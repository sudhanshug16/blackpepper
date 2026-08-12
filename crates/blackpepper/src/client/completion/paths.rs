//! Hierarchical completion for the static portions of command syntax.

use super::{runnable_prefix, Candidate};
use crate::client::{catalog, ClientState};

pub(super) fn command_paths(state: &ClientState, input: &str, open: bool) -> Vec<Candidate> {
    let normalized = input.split_whitespace().collect::<Vec<_>>().join(" ");
    let input_words = normalized.split_whitespace().collect::<Vec<_>>();
    let mut routes = catalog::entries(state)
        .into_iter()
        .map(|entry| {
            let (value, expects_more) = runnable_prefix(entry.syntax);
            (
                value
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect::<Vec<_>>(),
                expects_more,
                entry.note,
                entry.available,
            )
        })
        .collect::<Vec<_>>();
    // Keep disabled paths discoverable, but do not let them crowd runnable
    // commands out of the bounded opening palette.
    routes.sort_by_key(|(_, _, _, available)| !*available);

    // Once a complete branch token has been typed (`host`, `workspace`), show
    // its children immediately. A partial token (`ho`) stays at this level.
    let exact_branch = !open
        && !input_words.is_empty()
        && routes.iter().any(|(route, expects_more, _, _)| {
            starts_with(route, &input_words)
                && (route.len() > input_words.len()
                    || route.len() == input_words.len() && *expects_more)
        });
    let (completed, partial) = if open || exact_branch {
        (input_words.as_slice(), "")
    } else {
        input_words
            .split_last()
            .map_or((&[][..], ""), |(partial, completed)| (completed, *partial))
    };

    let mut candidates = Vec::new();
    if exact_branch {
        for (route, expects_more, note, _) in &routes {
            if route.len() == completed.len()
                && starts_with(route, completed)
                && !candidates
                    .iter()
                    .any(|candidate: &Candidate| candidate.value == normalized)
            {
                candidates.push(Candidate {
                    value: normalized.clone(),
                    note: note.clone(),
                    expects_more: *expects_more,
                });
            }
        }
    }

    for (route, _, _, _) in &routes {
        if !starts_with(route, completed) {
            continue;
        }
        let Some(next) = route.get(completed.len()) else {
            continue;
        };
        if !next.starts_with(partial) {
            continue;
        }
        let value = completed
            .iter()
            .copied()
            .chain(std::iter::once(next.as_str()))
            .collect::<Vec<_>>()
            .join(" ");
        if candidates.iter().any(|candidate| candidate.value == value) {
            continue;
        }
        let value_prefix = value.split_whitespace().collect::<Vec<_>>();
        let matching = routes
            .iter()
            .filter(|(candidate, _, _, _)| starts_with(candidate, &value_prefix))
            .collect::<Vec<_>>();
        let exact = matching
            .iter()
            .find(|(candidate, _, _, _)| candidate.len() == completed.len() + 1);
        let expects_more = matching
            .iter()
            .any(|(candidate, more, _, _)| candidate.len() > completed.len() + 1 || *more);
        let note = exact.map_or_else(
            || format!("{} commands", matching.len()),
            |(_, _, note, _)| note.clone(),
        );
        candidates.push(Candidate {
            value,
            note,
            expects_more,
        });
    }
    candidates
}

fn starts_with(route: &[String], prefix: &[&str]) -> bool {
    route.len() >= prefix.len()
        && route
            .iter()
            .zip(prefix)
            .all(|(route, prefix)| route.as_str() == *prefix)
}
