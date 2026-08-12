use std::borrow::Cow;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::core::AgentRunId;

use super::{blocker_manifest, BlockerManifestError, Provider};

/// Confidence assigned by a pinned blocker rule.
///
/// This is manifest metadata, not a score inferred from terminal contents.
#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockerConfidence {
    Medium,
    High,
}

/// Redacted explanation for a visible blocker match.
///
/// It deliberately contains no matched text, viewport, prompt, command, or
/// filesystem path.
#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BlockerExplain {
    pub provider: Provider,
    pub manifest_version: String,
    pub rule_id: String,
    pub confidence: BlockerConfidence,
    pub priority: i32,
}

/// Terminal data used locally by the matcher. This type is borrowed and is
/// never stored in a snapshot or explanation.
#[derive(Debug, Clone, Copy)]
pub struct BlockerInput<'a> {
    pub viewport: &'a str,
    pub terminal_title: Option<&'a str>,
}

/// Redacted result sent from a viewport watcher to a status tracker.
#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BlockerObservation {
    pub run_id: AgentRunId,
    pub sequence: u64,
    pub observed_at_ms: u64,
    pub blocker: Option<BlockerExplain>,
}

impl BlockerObservation {
    pub fn evaluate(
        run_id: AgentRunId,
        sequence: u64,
        observed_at_ms: u64,
        overlay: &BlockerOverlay,
        input: BlockerInput<'_>,
    ) -> Self {
        Self {
            run_id,
            sequence,
            observed_at_ms,
            blocker: overlay.evaluate(input),
        }
    }
}

#[derive(Debug)]
pub struct BlockerOverlay {
    provider: Provider,
    version: String,
    rules: Vec<CompiledRule>,
}

impl BlockerOverlay {
    pub fn from_toml(input: &str) -> Result<Self, BlockerManifestError> {
        blocker_manifest::parse(input)
    }

    pub fn bundled(provider: Provider) -> Result<Self, BlockerManifestError> {
        let input = match provider {
            Provider::Codex => include_str!("../../assets/agent-status/codex.toml"),
            Provider::Claude => include_str!("../../assets/agent-status/claude.toml"),
            Provider::OpenCode => include_str!("../../assets/agent-status/opencode.toml"),
        };
        let overlay = Self::from_toml(input)?;
        if overlay.provider != provider {
            return Err(BlockerManifestError::ProviderMismatch {
                expected: provider,
                actual: overlay.provider,
            });
        }
        Ok(overlay)
    }

    pub const fn provider(&self) -> Provider {
        self.provider
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn evaluate(&self, input: BlockerInput<'_>) -> Option<BlockerExplain> {
        self.rules
            .iter()
            .filter(|rule| rule.matches(input))
            .max_by_key(|rule| rule.priority)
            .map(|rule| BlockerExplain {
                provider: self.provider,
                manifest_version: self.version.clone(),
                rule_id: rule.id.clone(),
                confidence: rule.confidence,
                priority: rule.priority,
            })
    }

    pub(super) fn from_parts(
        provider: Provider,
        version: String,
        rules: Vec<CompiledRule>,
    ) -> Self {
        Self {
            provider,
            version,
            rules,
        }
    }
}

#[derive(Debug)]
pub(super) struct CompiledRule {
    pub(super) id: String,
    pub(super) confidence: BlockerConfidence,
    pub(super) priority: i32,
    pub(super) region: Region,
    pub(super) contains_all: Vec<String>,
    pub(super) contains_any: Vec<String>,
    pub(super) regex_all: Vec<Regex>,
    pub(super) regex_any: Vec<Regex>,
    pub(super) not_contains: Vec<String>,
    pub(super) not_regex: Vec<Regex>,
}

impl CompiledRule {
    fn matches(&self, input: BlockerInput<'_>) -> bool {
        let text = self.region.select(input);
        let lower = text.to_lowercase();

        if !self
            .contains_all
            .iter()
            .all(|needle| lower.contains(needle))
            || !self.regex_all.iter().all(|pattern| pattern.is_match(&text))
        {
            return false;
        }

        let has_any = !self.contains_any.is_empty() || !self.regex_any.is_empty();
        if has_any
            && !self
                .contains_any
                .iter()
                .any(|needle| lower.contains(needle))
            && !self.regex_any.iter().any(|pattern| pattern.is_match(&text))
        {
            return false;
        }

        !self
            .not_contains
            .iter()
            .any(|needle| lower.contains(needle))
            && !self.not_regex.iter().any(|pattern| pattern.is_match(&text))
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum Region {
    Viewport,
    BottomLines(usize),
    TopLines(usize),
    TerminalTitle,
}

impl Region {
    fn select<'a>(self, input: BlockerInput<'a>) -> Cow<'a, str> {
        match self {
            Self::Viewport => Cow::Borrowed(input.viewport),
            Self::TerminalTitle => Cow::Borrowed(input.terminal_title.unwrap_or("")),
            Self::BottomLines(lines) => Cow::Owned(select_bottom(input.viewport, lines)),
            Self::TopLines(lines) => Cow::Owned(
                input
                    .viewport
                    .lines()
                    .take(lines)
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
        }
    }
}

fn select_bottom(input: &str, lines: usize) -> String {
    let selected = input.lines().rev().take(lines).collect::<Vec<_>>();
    selected.into_iter().rev().collect::<Vec<_>>().join("\n")
}
