use nucleo_matcher::{
    pattern::{CaseMatching, Normalization, Pattern},
    Config, Matcher,
};

use super::{
    model::{ExpandedTerm, ExpandedTermGroup, ResolvedQuery},
    types::{term_id_from_usize, TermId},
    FullTextDatastore,
};

/// Maximum fuzzy-expanded catalog terms retained per positive full-text atom.
const FUZZY_POSITIVE_TERM_LIMIT: usize = 50;
/// Keep fuzzy terms whose score is at least this fraction of the atom's best.
const FUZZY_SCORE_FLOOR_NUMERATOR: u32 = 7;
const FUZZY_SCORE_FLOOR_DENOMINATOR: u32 = 10;

impl FullTextDatastore {
    /// Expands each parsed query atom against the indexed word catalog.
    pub(super) fn resolve_query(&self, query: &str) -> Option<ResolvedQuery> {
        if self.fuzzy_search {
            self.resolve_query_fuzzy(query)
        } else {
            self.resolve_query_exact(query)
        }
    }

    /// Returns positive expanded terms that are present in the matched record.
    pub(super) fn matched_terms_for_record(
        &self,
        record_terms: &[TermId],
        positive_groups: &[ExpandedTermGroup],
    ) -> Vec<String> {
        let mut matched_terms = Vec::<String>::new();
        for group in positive_groups {
            for term in group {
                if record_terms.binary_search(&term.term_id).is_ok() {
                    if let Some(catalog_term) = self.terms.get(term.term_id as usize) {
                        matched_terms.push(catalog_term.text.clone());
                    }
                }
            }
        }

        matched_terms.sort_unstable();
        matched_terms.dedup();
        matched_terms
    }

    /// Fuzzy query expansion using nucleo_matcher for approximate term matching.
    fn resolve_query_fuzzy(&self, query: &str) -> Option<ResolvedQuery> {
        let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
        if pattern.atoms.is_empty() {
            return None;
        }

        let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
        let mut positive_groups = Vec::<ExpandedTermGroup>::new();
        let mut negative_groups = Vec::<ExpandedTermGroup>::new();

        for atom in pattern.atoms {
            let negative = atom.negative;
            let mut positive_atom = atom;
            positive_atom.negative = false;

            let mut expanded_terms = self
                .terms
                .iter()
                .enumerate()
                .filter(|(term_idx, _)| {
                    self.postings
                        .get(*term_idx)
                        .is_some_and(|postings| !postings.is_empty())
                })
                .filter_map(|(term_idx, term)| {
                    let score =
                        positive_atom.score(term.matcher_text.as_utf32_str(), &mut matcher)?;
                    Some(ExpandedTerm {
                        term_id: term_id_from_usize(term_idx),
                        fuzzy_score: score as u32,
                    })
                })
                .collect::<Vec<_>>();

            expanded_terms = filter_fuzzy_expanded_terms(expanded_terms, negative);
            expanded_terms.sort_unstable_by_key(|term| term.term_id);
            expanded_terms.dedup_by_key(|term| term.term_id);

            if negative {
                negative_groups.push(expanded_terms);
            } else if expanded_terms.is_empty() {
                return None;
            } else {
                positive_groups.push(expanded_terms);
            }
        }

        Some((positive_groups, negative_groups))
    }

    /// Exact query expansion: exact lookup, prefix, or substring comparisons.
    fn resolve_query_exact(&self, query: &str) -> Option<ResolvedQuery> {
        let atoms = self.parse_query_atoms(query);
        if atoms.is_empty() {
            return None;
        }

        let mut positive_groups = Vec::<ExpandedTermGroup>::new();
        let mut negative_groups = Vec::<ExpandedTermGroup>::new();

        for (negative, mode, text) in atoms {
            let expanded_terms = self.expand_terms_exact(mode, &text);

            if negative {
                negative_groups.push(expanded_terms);
            } else if expanded_terms.is_empty() {
                return None;
            } else {
                positive_groups.push(expanded_terms);
            }
        }

        Some((positive_groups, negative_groups))
    }

    /// Parses query atoms with optional negation, prefix, and substring markers.
    fn parse_query_atoms(&self, query: &str) -> Vec<(bool, &str, String)> {
        let mut atoms = Vec::new();
        for raw in query.split_whitespace() {
            if raw.is_empty() {
                continue;
            }

            let mut text = raw;
            let mut negative = false;
            if text.starts_with('!') {
                negative = true;
                text = &text[1..];
            }
            if text.is_empty() {
                continue;
            }

            let mode = if text.starts_with('^') {
                text = &text[1..];
                if text.is_empty() {
                    continue;
                }
                "prefix"
            } else if text.starts_with('\'') {
                text = &text[1..];
                if text.is_empty() {
                    continue;
                }
                "substring"
            } else {
                "exact"
            };

            let mut lowered = String::new();
            for ch in text.chars() {
                for lc in ch.to_lowercase() {
                    lowered.push(lc);
                }
            }
            atoms.push((negative, mode, lowered));
        }
        atoms
    }

    /// Collects matching catalog terms using exact, prefix, or substring logic.
    fn expand_terms_exact(&self, mode: &str, text: &str) -> Vec<ExpandedTerm> {
        match mode {
            "prefix" => self
                .term_to_id
                .iter()
                .filter(|(term, _)| term.starts_with(text))
                .map(|(_, &term_id)| ExpandedTerm {
                    term_id,
                    fuzzy_score: 0,
                })
                .collect(),
            "substring" => self
                .term_to_id
                .iter()
                .filter(|(term, _)| term.contains(text))
                .map(|(_, &term_id)| ExpandedTerm {
                    term_id,
                    fuzzy_score: 0,
                })
                .collect(),
            _ => self
                .term_to_id
                .get(text)
                .map(|&term_id| {
                    vec![ExpandedTerm {
                        term_id,
                        fuzzy_score: 0,
                    }]
                })
                .unwrap_or_default(),
        }
    }
}

/// Applies the fuzzy quality floor and caps positive query expansion breadth.
pub(super) fn filter_fuzzy_expanded_terms(
    mut terms: Vec<ExpandedTerm>,
    negative: bool,
) -> Vec<ExpandedTerm> {
    let Some(best_score) = terms.iter().map(|term| term.fuzzy_score).max() else {
        return terms;
    };

    let score_floor =
        (best_score * FUZZY_SCORE_FLOOR_NUMERATOR).div_ceil(FUZZY_SCORE_FLOOR_DENOMINATOR);
    terms.retain(|term| term.fuzzy_score >= score_floor);
    if !negative && terms.len() > FUZZY_POSITIVE_TERM_LIMIT {
        terms.sort_unstable_by(|a, b| {
            b.fuzzy_score
                .cmp(&a.fuzzy_score)
                .then_with(|| a.term_id.cmp(&b.term_id))
        });
        terms.truncate(FUZZY_POSITIVE_TERM_LIMIT);
    }

    terms
}
