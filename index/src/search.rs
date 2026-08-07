use crate::indexer::FlagEntry;

/// Search the flag index for a query like "dry-run" or "--dry-run".
///
/// Matching is token-aware, not raw substring: flags are split on their
/// natural delimiters (-, :, =, _, .), and the query must match a whole
/// token to count. This matters in practice — a raw substring search for
/// "all" would otherwise match "-XX:AllocateHeapAt=path" (since "all" is
/// a fragment of "Allocate"), which is technically true but useless
/// noise. Token matching correctly finds "--all" and "--almost-all"
/// (both have "all" as a standalone token) while excluding "AllocateHeapAt".
///
/// Ranked in two tiers: exact whole-flag match first, then any other
/// flag containing the query as a token, sorted by page name within
/// each tier.
pub fn search<'a>(entries: &'a [FlagEntry], query: &str) -> Vec<&'a FlagEntry> {
    let normalized_query = normalize(query);
    if normalized_query.is_empty() {
        return Vec::new();
    }

    let mut exact: Vec<&FlagEntry> = Vec::new();
    let mut token_match: Vec<&FlagEntry> = Vec::new();

    for entry in entries {
        let normalized_flag = normalize(&entry.flag);
        if normalized_flag == normalized_query {
            exact.push(entry);
        } else if tokenize(&normalized_flag).any(|t| t == normalized_query) {
            token_match.push(entry);
        }
    }

    exact.sort_by(|a, b| (&a.name, &a.section).cmp(&(&b.name, &b.section)));
    token_match.sort_by(|a, b| (&a.name, &a.section).cmp(&(&b.name, &b.section)));

    exact.into_iter().chain(token_match).collect()
}

fn normalize(s: &str) -> String {
    s.trim().trim_start_matches('-').to_lowercase()
}

fn tokenize(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
}