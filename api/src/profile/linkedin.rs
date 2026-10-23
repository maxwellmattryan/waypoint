//! `POST /profile/linkedin`: imports a LinkedIn data export. The body is the raw `.zip`
//! (`Content-Type: application/zip`), not multipart, so [`NotCrossSite`] guards it like any
//! other changing route.
//!
//! The archive is read in memory and never extracted, so entry paths are only names to
//! match. `Positions.csv`, `Skills.csv` and `Education.csv` are found by basename, at any
//! depth and in any case; every other entry is ignored. Each one is read through a size cap
//! whatever size its header declares. The rows are stored confirmed, with source
//! `linkedin`, in one transaction.

use std::io::{Cursor, Read};

use rocket::State;
use rocket::data::{Data, ToByteUnit};
use rocket::http::Status;
use rocket::request::{FromRequest, Outcome, Request};
use rocket::serde::json::Json;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use super::{ProfileEntry, ProfileImportSummary, store};
use crate::applications::{ApiError, FieldError, ValidationErrors, internal, text};
use crate::host::{LocalHost, NotCrossSite};
use crate::memories::{MemorySource, MemoryStatus, SourceKind};

/// The largest archive an import accepts: 20 MiB.
pub const MAX_ZIP_BYTES: usize = 20 * 1024 * 1024;
/// The most one CSV entry may inflate to: 5 MiB.
pub const MAX_CSV_BYTES: usize = 5 * 1024 * 1024;
/// Most characters in `filename`.
pub const MAX_FILENAME: usize = 255;

const NOT_ZIP: &str = "Must be a LinkedIn data export (.zip)";
const NOTHING_FOUND: &str = "No Positions, Skills or Education CSV found in the archive";

// Documents a raw zip body as a binary string rather than utoipa's array of integers.
/// Raw zip bytes.
#[derive(utoipa::ToSchema)]
#[schema(value_type = String, format = Binary)]
pub struct ZipBytes(pub Vec<u8>);

/// Admits only a `Content-Type: application/zip` body. Taken after [`NotCrossSite`], so a
/// form or multipart body is a 403 and any other type (or none) is a 415.
pub struct ZipContent;

#[rocket::async_trait]
impl<'r> FromRequest<'r> for ZipContent {
    type Error = ();

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, ()> {
        if req.content_type().is_some_and(|ct| ct.is_zip()) {
            Outcome::Success(Self)
        } else {
            Outcome::Error((Status::UnsupportedMediaType, ()))
        }
    }
}

/// One of the three export files.
struct ExportFile {
    /// The entry's name as LinkedIn writes it.
    name: &'static str,
    /// Columns the header row must have; their values come first in each row.
    required: &'static [&'static str],
    optional: &'static [&'static str],
}

const POSITIONS: ExportFile = ExportFile {
    name: "Positions.csv",
    required: &["Company Name", "Title"],
    optional: &["Location", "Started On", "Finished On", "Description"],
};
const SKILLS: ExportFile = ExportFile {
    name: "Skills.csv",
    required: &["Name"],
    optional: &[],
};
const EDUCATION: ExportFile = ExportFile {
    name: "Education.csv",
    required: &["School Name"],
    optional: &["Degree Name", "Start Date", "End Date", "Notes"],
};

/// The entries read from an archive, positions then skills then education, each in file
/// order, and how many rows were missing a required value.
#[derive(Debug, Default, PartialEq)]
pub struct Parsed {
    pub entries: Vec<ProfileEntry>,
    pub skipped: u32,
}

/// Why an archive can't be imported. Each is a 422 on `file`.
#[derive(Debug, PartialEq)]
pub enum ArchiveError {
    /// Not a zip archive, or a matched entry can't be read.
    NotZip,
    /// This entry inflates past [`MAX_CSV_BYTES`].
    TooLarge(&'static str),
    /// None of the three files is present with a recognisable header.
    NothingFound,
}

impl ArchiveError {
    fn message(&self) -> String {
        match self {
            Self::NotZip => NOT_ZIP.into(),
            Self::TooLarge(name) => format!("{name} is too large"),
            Self::NothingFound => NOTHING_FOUND.into(),
        }
    }
}

/// Reads the three export files out of `bytes`.
pub fn parse_archive(bytes: &[u8]) -> Result<Parsed, ArchiveError> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| ArchiveError::NotZip)?;
    let files = [&POSITIONS, &SKILLS, &EDUCATION];
    let mut found: [Option<Vec<u8>>; 3] = [None, None, None];
    for index in 0..archive.len() {
        let Some(name) = archive.name_for_index(index) else {
            continue;
        };
        let basename = name.rsplit('/').next().unwrap_or(name);
        let Some(slot) = files
            .iter()
            .position(|file| basename.eq_ignore_ascii_case(file.name))
        else {
            continue;
        };
        if found[slot].is_some() {
            continue;
        }
        let entry = archive.by_index(index).map_err(|_| ArchiveError::NotZip)?;
        let mut content = Vec::new();
        entry
            .take(MAX_CSV_BYTES as u64 + 1)
            .read_to_end(&mut content)
            .map_err(|_| ArchiveError::NotZip)?;
        if content.len() > MAX_CSV_BYTES {
            return Err(ArchiveError::TooLarge(files[slot].name));
        }
        found[slot] = Some(content);
    }

    let mut parsed = Parsed::default();
    let mut recognised = false;
    for (file, content) in files.iter().zip(&found) {
        let Some(rows) = content.as_deref().and_then(|c| rows(c, file)) else {
            continue;
        };
        recognised = true;
        for row in rows {
            let cell = |i: usize| row.get(i).and_then(|v| v.as_deref());
            let entry = match file.name {
                "Positions.csv" => {
                    ProfileEntry::position(cell(0), cell(1), cell(2), cell(3), cell(4), cell(5))
                }
                "Skills.csv" => ProfileEntry::skill(cell(0)),
                _ => ProfileEntry::education(cell(0), cell(1), cell(2), cell(3), cell(4)),
            };
            match entry {
                Some(entry) => parsed.entries.push(entry),
                None => parsed.skipped += 1,
            }
        }
    }
    if recognised {
        Ok(parsed)
    } else {
        Err(ArchiveError::NothingFound)
    }
}

/// The data rows of one CSV, each holding the file's required then optional columns in
/// order (`None` for a missing cell or column), or `None` when no header row is found.
/// The header row is the first record whose cells include every required column, compared
/// trimmed and case-insensitively; records before it are ignored.
fn rows(content: &[u8], file: &ExportFile) -> Option<Vec<Vec<Option<String>>>> {
    let content = content.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(content);
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(content);
    let wanted: Vec<&str> = file.required.iter().chain(file.optional).copied().collect();
    let mut columns: Option<Vec<Option<usize>>> = None;
    let mut out = Vec::new();
    for record in reader.byte_records() {
        let Ok(record) = record else {
            continue;
        };
        let cells: Vec<String> = record
            .iter()
            .map(|cell| String::from_utf8_lossy(cell).into_owned())
            .collect();
        match &columns {
            None => {
                let index: Vec<Option<usize>> = wanted
                    .iter()
                    .map(|name| {
                        cells
                            .iter()
                            .position(|cell| cell.trim().eq_ignore_ascii_case(name))
                    })
                    .collect();
                if index[..file.required.len()].iter().all(Option::is_some) {
                    columns = Some(index);
                }
            }
            Some(index) => out.push(
                index
                    .iter()
                    .map(|i| i.and_then(|i| cells.get(i).cloned()))
                    .collect(),
            ),
        }
    }
    columns.map(|_| out)
}

fn file_error(message: String) -> ApiError {
    ApiError::Invalid(ValidationErrors {
        errors: vec![FieldError {
            field: "file".into(),
            message,
        }],
    })
}

/// Imports the positions, skills and education in a LinkedIn data export as confirmed
/// facts. Rows whose key is already stored are counted, not stored again.
#[utoipa::path(
    post,
    path = "/profile/linkedin",
    tag = "profile",
    operation_id = "import_linkedin",
    params(
        ("filename" = Option<String>, Query, description = "The uploaded file's name, at most 255 characters; stored as each memory's `source_label`"),
    ),
    request_body(content = inline(ZipBytes), content_type = "application/zip", description = "The raw LinkedIn data export archive, 1 byte to 20 MiB. `Positions.csv`, `Skills.csv` and `Education.csv` are read (at any depth, any case), each up to 5 MiB inflated; every other entry is ignored"),
    responses(
        (status = 200, description = "Every row is applied: new rows stored as confirmed `fact` memories with source `linkedin`", body = ProfileImportSummary),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 413, description = "The body is over 20 MiB"),
        (status = 415, description = "The body is not `application/zip`"),
        (status = 422, description = "`file` is empty, not a zip, has none of the three CSVs, or has a CSV over 5 MiB; or `filename` is too long. Nothing is stored", body = ValidationErrors),
        (status = 500, description = "The import could not be stored; nothing is stored"),
    )
)]
#[rocket::post("/profile/linkedin?<filename>", data = "<data>")]
pub async fn import(
    _host: LocalHost,
    _site: NotCrossSite,
    _zip: ZipContent,
    filename: Option<&str>,
    pool: &State<PgPool>,
    data: Data<'_>,
) -> Result<Json<ProfileImportSummary>, ApiError> {
    // An explicit `open` limit replaces Rocket's default `limits` for this body.
    let body = data
        .open((MAX_ZIP_BYTES + 1).bytes())
        .into_bytes()
        .await
        .map_err(|err| {
            rocket::warn!("profile: cannot read the upload: {}", err.kind());
            ApiError::BadRequest
        })?;
    if !body.is_complete() || body.len() > MAX_ZIP_BYTES {
        return Err(ApiError::TooLarge);
    }
    let bytes = body.into_inner();

    let mut errors = Vec::new();
    let filename = text(&mut errors, "filename", filename, MAX_FILENAME);
    if bytes.is_empty() {
        errors.push(FieldError {
            field: "file".into(),
            message: "Required".into(),
        });
    }
    if !errors.is_empty() {
        return Err(ApiError::Invalid(ValidationErrors { errors }));
    }

    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    let parsed = tokio::task::spawn_blocking(move || parse_archive(&bytes))
        .await
        .map_err(|err| {
            rocket::warn!("profile: the archive task failed: {err}");
            ApiError::Internal
        })?
        .map_err(|err| file_error(err.message()))?;

    let source = MemorySource {
        kind: SourceKind::Linkedin,
        reference: Some(sha256),
        label: filename,
    };
    let mut summary = ProfileImportSummary {
        skipped: parsed.skipped,
        ..ProfileImportSummary::default()
    };
    let mut tx = pool
        .begin()
        .await
        .map_err(internal("start a LinkedIn import"))?;
    store(
        &mut tx,
        &parsed.entries,
        MemoryStatus::Confirmed,
        &source,
        &mut summary,
    )
    .await
    .map_err(internal("store a LinkedIn import"))?;
    tx.commit()
        .await
        .map_err(internal("commit a LinkedIn import"))?;
    Ok(Json(summary))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use zip::write::SimpleFileOptions;

    use super::*;

    /// A zip of `(name, content)` entries, deflated.
    fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, content) in entries {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(content).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn skill(name: &str) -> ProfileEntry {
        ProfileEntry::skill(Some(name)).unwrap()
    }

    #[test]
    fn parses_export_csvs() {
        let positions = "\u{FEFF}Notes:\n\"LinkedIn preamble, with a comma\"\n\nCompany Name,Title,Description,Location,Started On,Finished On,Extra\nAcme,Engineer,\"Built, shipped\nand ran things\",Berlin,Jan 2020,,x\n ,Missing company,,,,,\nHooli,  Staff  ,,,Mar 2018,Dec 2019\n";
        let skills = "name\nRust\n\n  \nGo\n";
        let education = "School Name,Start Date,End Date,Notes,Degree Name,Activities\nMIT,2010,2014,Honours,BSc,Rowing\n,2001,2002,,,\n";
        let cases: Vec<(&str, Vec<u8>, Result<Parsed, ArchiveError>)> = vec![
            (
                "all three, nested and in any case",
                archive(&[
                    (
                        "Basic_LinkedInDataExport/POSITIONS.CSV",
                        positions.as_bytes(),
                    ),
                    ("Basic_LinkedInDataExport/Skills.csv", skills.as_bytes()),
                    ("Education.csv", education.as_bytes()),
                    ("Profile.csv", b"First Name\nAda\n"),
                    ("Basic_LinkedInDataExport/", b""),
                ]),
                Ok(Parsed {
                    entries: vec![
                        ProfileEntry::position(
                            Some("Acme"),
                            Some("Engineer"),
                            Some("Berlin"),
                            Some("Jan 2020"),
                            None,
                            Some("Built, shipped\nand ran things"),
                        )
                        .unwrap(),
                        ProfileEntry::position(
                            Some("Hooli"),
                            Some("Staff"),
                            None,
                            Some("Mar 2018"),
                            Some("Dec 2019"),
                            None,
                        )
                        .unwrap(),
                        skill("Rust"),
                        skill("Go"),
                        ProfileEntry::education(
                            Some("MIT"),
                            Some("BSc"),
                            Some("2010"),
                            Some("2014"),
                            Some("Honours"),
                        )
                        .unwrap(),
                    ],
                    // The blank company, the whitespace-only skill row and the blank school.
                    skipped: 3,
                }),
            ),
            (
                "skills only",
                archive(&[("skills.csv", b"Name\nRust\n")]),
                Ok(Parsed {
                    entries: vec![skill("Rust")],
                    skipped: 0,
                }),
            ),
            (
                "the first matching entry wins",
                archive(&[
                    ("a/Skills.csv", b"Name\nRust\n"),
                    ("b/Skills.csv", b"Name\nGo\n"),
                ]),
                Ok(Parsed {
                    entries: vec![skill("Rust")],
                    skipped: 0,
                }),
            ),
            (
                "an unrecognised header beside a good file",
                archive(&[
                    ("Positions.csv", b"Employer,Role\nAcme,Dev\n"),
                    ("Skills.csv", b"Name\nRust\n"),
                ]),
                Ok(Parsed {
                    entries: vec![skill("Rust")],
                    skipped: 0,
                }),
            ),
            (
                "no recognisable header",
                archive(&[("Positions.csv", b"Employer,Role\nAcme,Dev\n")]),
                Err(ArchiveError::NothingFound),
            ),
            (
                "none of the files",
                archive(&[("Profile.csv", b"First Name\nAda\n")]),
                Err(ArchiveError::NothingFound),
            ),
            (
                "not a zip",
                b"Name\nRust\n".to_vec(),
                Err(ArchiveError::NotZip),
            ),
            (
                "an entry over the cap",
                archive(&[("Skills.csv", &vec![b'a'; MAX_CSV_BYTES + 1])]),
                Err(ArchiveError::TooLarge("Skills.csv")),
            ),
        ];
        for (name, bytes, want) in cases {
            assert_eq!(parse_archive(&bytes), want, "{name}");
        }
    }
}
