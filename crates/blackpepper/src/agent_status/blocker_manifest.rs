use std::{collections::HashSet, fmt};

use regex::Regex;
use serde::Deserialize;

use super::{
    blocker::BlockerConfidence, blocker::BlockerOverlay, blocker::CompiledRule, blocker::Region,
    Provider,
};

const SCHEMA_VERSION: u32 = 1;
const MAX_MANIFEST_BYTES: usize = 128 * 1024;
const MAX_RULES: usize = 64;
const MAX_LINES: usize = 200;
const MAX_MATCHERS_PER_RULE: usize = 64;
const MAX_MATCHER_CHARS: usize = 512;

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum BlockerManifestError {
    TooLarge,
    InvalidToml(String),
    UnsupportedSchema(u32),
    InvalidVersion,
    NoRules,
    TooManyRules,
    InvalidRule {
        rule_id: String,
        reason: &'static str,
    },
    InvalidRegex {
        rule_id: String,
        pattern: String,
    },
    DuplicateRule(String),
    ProviderMismatch {
        expected: Provider,
        actual: Provider,
    },
}

impl fmt::Display for BlockerManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => formatter.write_str("blocker manifest is too large"),
            Self::InvalidToml(error) => write!(formatter, "invalid blocker manifest TOML: {error}"),
            Self::UnsupportedSchema(version) => {
                write!(formatter, "unsupported blocker manifest schema: {version}")
            }
            Self::InvalidVersion => formatter.write_str("manifest version must be dotted numeric"),
            Self::NoRules => formatter.write_str("blocker manifest has no rules"),
            Self::TooManyRules => formatter.write_str("blocker manifest has too many rules"),
            Self::InvalidRule { rule_id, reason } => {
                write!(formatter, "invalid blocker rule {rule_id}: {reason}")
            }
            Self::InvalidRegex { rule_id, pattern } => {
                write!(
                    formatter,
                    "invalid regex in blocker rule {rule_id}: {pattern}"
                )
            }
            Self::DuplicateRule(rule_id) => write!(formatter, "duplicate blocker rule: {rule_id}"),
            Self::ProviderMismatch { expected, actual } => {
                write!(
                    formatter,
                    "expected {expected} blocker manifest, found {actual}"
                )
            }
        }
    }
}

impl std::error::Error for BlockerManifestError {}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    schema_version: u32,
    provider: Provider,
    version: String,
    blockers: Vec<RawRule>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRule {
    id: String,
    confidence: BlockerConfidence,
    #[serde(default)]
    priority: i32,
    #[serde(default)]
    region: RawRegion,
    #[serde(default)]
    contains_all: Vec<String>,
    #[serde(default)]
    contains_any: Vec<String>,
    #[serde(default)]
    regex_all: Vec<String>,
    #[serde(default)]
    regex_any: Vec<String>,
    #[serde(default)]
    not_contains: Vec<String>,
    #[serde(default)]
    not_regex: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRegion {
    #[serde(default)]
    kind: RegionKind,
    lines: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RegionKind {
    #[default]
    Viewport,
    BottomLines,
    TopLines,
    TerminalTitle,
}

pub(super) fn parse(input: &str) -> Result<BlockerOverlay, BlockerManifestError> {
    if input.len() > MAX_MANIFEST_BYTES {
        return Err(BlockerManifestError::TooLarge);
    }
    let manifest: RawManifest = toml::from_str(input)
        .map_err(|error| BlockerManifestError::InvalidToml(error.to_string()))?;
    if manifest.schema_version != SCHEMA_VERSION {
        return Err(BlockerManifestError::UnsupportedSchema(
            manifest.schema_version,
        ));
    }
    if !valid_version(&manifest.version) {
        return Err(BlockerManifestError::InvalidVersion);
    }
    if manifest.blockers.is_empty() {
        return Err(BlockerManifestError::NoRules);
    }
    if manifest.blockers.len() > MAX_RULES {
        return Err(BlockerManifestError::TooManyRules);
    }

    let mut ids = HashSet::new();
    let mut rules = Vec::with_capacity(manifest.blockers.len());
    for rule in manifest.blockers {
        if !ids.insert(rule.id.clone()) {
            return Err(BlockerManifestError::DuplicateRule(rule.id));
        }
        rules.push(compile_rule(rule)?);
    }

    Ok(BlockerOverlay::from_parts(
        manifest.provider,
        manifest.version,
        rules,
    ))
}

fn compile_rule(raw: RawRule) -> Result<CompiledRule, BlockerManifestError> {
    validate_rule(&raw)?;
    Ok(CompiledRule {
        id: raw.id.clone(),
        confidence: raw.confidence,
        priority: raw.priority,
        region: compile_region(&raw.id, raw.region)?,
        contains_all: lower(raw.contains_all),
        contains_any: lower(raw.contains_any),
        regex_all: compile_regexes(&raw.id, raw.regex_all)?,
        regex_any: compile_regexes(&raw.id, raw.regex_any)?,
        not_contains: lower(raw.not_contains),
        not_regex: compile_regexes(&raw.id, raw.not_regex)?,
    })
}

fn validate_rule(rule: &RawRule) -> Result<(), BlockerManifestError> {
    let invalid_id = rule.id.is_empty()
        || rule.id.len() > 64
        || !rule.id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_-".contains(&byte)
        });
    if invalid_id {
        return invalid_rule(&rule.id, "id must use lowercase ASCII, digits, '-' or '_'");
    }

    let groups = [
        &rule.contains_all,
        &rule.contains_any,
        &rule.regex_all,
        &rule.regex_any,
        &rule.not_contains,
        &rule.not_regex,
    ];
    let matcher_count = groups.iter().map(|group| group.len()).sum::<usize>();
    if matcher_count > MAX_MATCHERS_PER_RULE {
        return invalid_rule(&rule.id, "too many matchers");
    }
    if groups
        .iter()
        .flat_map(|group| group.iter())
        .any(|matcher| matcher.is_empty() || matcher.len() > MAX_MATCHER_CHARS)
    {
        return invalid_rule(&rule.id, "matchers must be non-empty and at most 512 bytes");
    }
    let has_positive = !rule.contains_all.is_empty()
        || !rule.contains_any.is_empty()
        || !rule.regex_all.is_empty()
        || !rule.regex_any.is_empty();
    if !has_positive {
        return invalid_rule(&rule.id, "at least one positive matcher is required");
    }
    Ok(())
}

fn compile_region(rule_id: &str, raw: RawRegion) -> Result<Region, BlockerManifestError> {
    match (raw.kind, raw.lines) {
        (RegionKind::Viewport, None) => Ok(Region::Viewport),
        (RegionKind::TerminalTitle, None) => Ok(Region::TerminalTitle),
        (RegionKind::BottomLines, Some(lines)) if (1..=MAX_LINES).contains(&lines) => {
            Ok(Region::BottomLines(lines))
        }
        (RegionKind::TopLines, Some(lines)) if (1..=MAX_LINES).contains(&lines) => {
            Ok(Region::TopLines(lines))
        }
        _ => invalid_rule(
            rule_id,
            "region lines are missing, unexpected, or out of range",
        ),
    }
}

fn compile_regexes(
    rule_id: &str,
    patterns: Vec<String>,
) -> Result<Vec<Regex>, BlockerManifestError> {
    patterns
        .into_iter()
        .map(|pattern| {
            Regex::new(&pattern).map_err(|_| BlockerManifestError::InvalidRegex {
                rule_id: rule_id.to_string(),
                pattern,
            })
        })
        .collect()
}

fn lower(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.to_lowercase())
        .collect()
}

fn valid_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 64
        && version
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn invalid_rule<T>(rule_id: &str, reason: &'static str) -> Result<T, BlockerManifestError> {
    Err(BlockerManifestError::InvalidRule {
        rule_id: rule_id.to_string(),
        reason,
    })
}
