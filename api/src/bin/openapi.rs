//! Prints the API's OpenAPI document. `scripts/gen-api.sh` writes it to `api/openapi.json`.
//! Needs no database, `.env` or `claude` binary.

fn main() {
    print!("{}", waypoint_api::openapi::to_json());
}
