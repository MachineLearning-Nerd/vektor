use thiserror::Error;

pub type Result<T> = std::result::Result<T, VektorError>;

#[derive(Debug, Error)]
pub enum VektorError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("config error: {0}")]
    Config(String),

    #[error("not implemented: {0}")]
    NotImplemented(&'static str),

    #[error("MCP protocol error: {0}")]
    Mcp(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_io_error() {
        let error: VektorError =
            std::io::Error::new(std::io::ErrorKind::NotFound, "missing").into();

        assert!(error.to_string().contains("IO error"));
    }

    #[test]
    fn display_config_error() {
        let error = VektorError::Config("bad value".into());

        assert_eq!(error.to_string(), "config error: bad value");
    }

    #[test]
    fn display_mcp_error() {
        let error = VektorError::Mcp("invalid request".into());

        assert_eq!(error.to_string(), "MCP protocol error: invalid request");
    }

    #[test]
    fn result_alias_uses_vektor_error() {
        let result: Result<()> = Err(VektorError::NotImplemented("test"));

        assert!(matches!(result, Err(VektorError::NotImplemented("test"))));
    }
}
