// Tag normalisation, the same rule as `normalise_tag(s)` in api/src/memories.rs, so a tag the
// UI shows or filters by is the tag the API stores.

/** Trims a tag, lowercases it and collapses each run of whitespace to one space. */
export function normaliseTag(tag: string): string {
	return tag.trim().split(/\s+/).join(' ').toLowerCase();
}

/** Normalises every tag, drops blanks and drops duplicates, keeping the first occurrence. */
export function normaliseTags(tags: readonly string[]): string[] {
	const out: string[] = [];
	for (const raw of tags) {
		const tag = normaliseTag(raw);
		if (tag && !out.includes(tag)) out.push(tag);
	}
	return out;
}
