use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use uuid::Uuid;

macro_rules! uuid_id {
    ($name:ident) => {
        #[doc = concat!("Stable UUID identifier for a `", stringify!($name), "`.")]
        #[derive(
            Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(value).map(Self)
            }
        }

        impl From<Uuid> for $name {
            fn from(value: Uuid) -> Self {
                Self(value)
            }
        }

        impl From<$name> for Uuid {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

uuid_id!(HostId);
uuid_id!(WorkspaceId);
uuid_id!(SessionId);
uuid_id!(PaneId);
uuid_id!(RepositoryId);
uuid_id!(AgentRunId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_through_text_and_json() {
        let id = AgentRunId::new();
        assert_eq!(id.to_string().parse::<AgentRunId>().unwrap(), id);

        let encoded = serde_json::to_string(&id).unwrap();
        assert_eq!(serde_json::from_str::<AgentRunId>(&encoded).unwrap(), id);
    }
}
