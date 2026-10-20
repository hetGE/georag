//! Manual retrieval depth tiers (handoff 7.6).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Quick,
    Optimal,
    Deep,
    Deeper,
    Ludicrous,
}

impl Tier {
    pub const ALL: [Tier; 5] = [
        Tier::Quick,
        Tier::Optimal,
        Tier::Deep,
        Tier::Deeper,
        Tier::Ludicrous,
    ];

    pub fn top_k_per_tag(self) -> usize {
        match self {
            Tier::Quick => 5,
            Tier::Optimal => 10,
            Tier::Deep => 20,
            Tier::Deeper => 50,
            Tier::Ludicrous => 100,
        }
    }

    pub fn max_chunks(self) -> usize {
        match self {
            Tier::Quick => 8,
            Tier::Optimal => 16,
            Tier::Deep => 32,
            Tier::Deeper => 80,
            Tier::Ludicrous => 200,
        }
    }

    pub fn n_ctx(self) -> u32 {
        match self {
            Tier::Quick | Tier::Optimal => 32_768,
            Tier::Deep | Tier::Deeper => 131_072,
            Tier::Ludicrous => 262_144,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Quick => "quick",
            Tier::Optimal => "optimal",
            Tier::Deep => "deep",
            Tier::Deeper => "deeper",
            Tier::Ludicrous => "ludicrous",
        }
    }
}

impl std::str::FromStr for Tier {
    type Err = crate::CoreError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Tier::ALL
            .iter()
            .copied()
            .find(|t| t.as_str() == s.trim().to_ascii_lowercase())
            .ok_or_else(|| {
                crate::CoreError::Invalid(format!(
                    "unknown depth tier {s:?}: use quick, optimal, deep, deeper or ludicrous"
                ))
            })
    }
}

impl std::fmt::Display for Tier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
