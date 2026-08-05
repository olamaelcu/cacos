use std::{
    fmt::{Display, Formatter},
    str::FromStr,
};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct Did(pub String);

impl Did {
    pub fn new(s: String) -> Self {
        Self(s)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl std::ops::Deref for Did {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<str> for Did {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Display for Did {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for Did {
    type Err = std::convert::Infallible;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.to_owned()))
    }
}

impl From<String> for Did {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl Serialize for Did {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Did {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(Self(s))
    }
}

impl From<Did> for sea_orm::Value {
    fn from(val: Did) -> Self {
        sea_orm::Value::String(Some(val.0))
    }
}

impl sea_orm::sea_query::ValueType for Did {
    fn try_from(v: sea_orm::Value) -> Result<Self, sea_orm::sea_query::ValueTypeErr> {
        match v {
            sea_orm::Value::String(Some(s)) => Ok(Self(s)),
            _ => Err(sea_orm::sea_query::ValueTypeErr),
        }
    }

    fn type_name() -> String {
        "Did".to_owned()
    }

    fn array_type() -> sea_orm::sea_query::ArrayType {
        sea_orm::sea_query::ArrayType::String
    }

    fn column_type() -> sea_orm::sea_query::ColumnType {
        sea_orm::sea_query::ColumnType::Text
    }
}

impl sea_orm::TryGetable for Did {
    fn try_get_by<I: sea_orm::ColIdx>(
        res: &sea_orm::QueryResult,
        index: I,
    ) -> Result<Self, sea_orm::TryGetError> {
        <String as sea_orm::TryGetable>::try_get_by(res, index).map(Self)
    }
}

impl sea_orm::sea_query::Nullable for Did {
    fn null() -> sea_orm::Value {
        sea_orm::Value::String(None)
    }
}

impl sea_orm::TryFromU64 for Did {
    fn try_from_u64(_: u64) -> Result<Self, sea_orm::DbErr> {
        Err(sea_orm::DbErr::ConvertFromU64("Did"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_did_display() {
        let did = Did::new("did:plc:abc123".into());
        assert_eq!(did.to_string(), "did:plc:abc123");
    }

    #[test]
    fn test_did_from_str() {
        let did: Did = "did:plc:abc123".parse().unwrap();
        assert_eq!(did.as_str(), "did:plc:abc123");
    }

    #[test]
    fn test_did_roundtrip() {
        let did = Did::new("did:web:example.com".into());
        let s = did.to_string();
        let restored: Did = s.parse().unwrap();
        assert_eq!(did, restored);
    }
}
