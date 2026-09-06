use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    Missing { key: &'static str },
    Invalid { key: &'static str, value: String },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { key } => write!(formatter, "missing configuration value: {key}"),
            Self::Invalid { key, value } => {
                write!(formatter, "invalid value for {key}: {value}")
            }
        }
    }
}

impl std::error::Error for ConfigError {}
