use tracing_subscriber::{EnvFilter, fmt, prelude::*};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Pretty,
    Json,
}

pub fn init(verbose: u8, format: Format) {
    let filter = EnvFilter::try_from_env("VEKTOR_LOG")
        .unwrap_or_else(|_| EnvFilter::new(format!("vektor={}", default_level_directive(verbose))));

    let registry = tracing_subscriber::registry().with(filter);

    match format {
        Format::Pretty => {
            registry
                .with(fmt::layer().with_writer(std::io::stderr))
                .init();
        }
        Format::Json => {
            registry
                .with(fmt::layer().json().with_writer(std::io::stderr))
                .init();
        }
    }
}

fn default_level_directive(verbose: u8) -> &'static str {
    match verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbose_count_maps_to_log_level() {
        assert_eq!(default_level_directive(0), "warn");
        assert_eq!(default_level_directive(1), "info");
        assert_eq!(default_level_directive(2), "debug");
        assert_eq!(default_level_directive(3), "trace");
        assert_eq!(default_level_directive(u8::MAX), "trace");
    }
}
