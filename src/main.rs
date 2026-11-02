use mdbook::errors::Error;
use mdbook::preprocess::{CmdPreprocessor, Preprocessor};
use mdbook_og_cards::cache::Cache;
use mdbook_og_cards::OgCardsPreprocessor;
use semver::{Version, VersionReq};
use std::{env, io, process};

fn main() {
    let mut args = env::args().skip(1);

    match args.next().as_deref() {
        Some("supports") => {
            let renderer = args.next().unwrap_or_default();
            let preprocessor = OgCardsPreprocessor::new();
            if preprocessor.supports_renderer(&renderer) {
                process::exit(0);
            } else {
                process::exit(1);
            }
        }
        Some("--clear-cache") | Some("clear-cache") => {
            let dir = args
                .next()
                .unwrap_or_else(|| ".cache/mdbook-og-cards".to_string());
            match Cache::clear(std::path::Path::new(&dir)) {
                Ok(()) => {
                    eprintln!("og-cards: cleared cache directory {dir}");
                    process::exit(0);
                }
                Err(e) => {
                    eprintln!("og-cards: failed to clear cache directory {dir}: {e}");
                    process::exit(1);
                }
            }
        }
        _ => {
            let preprocessor = OgCardsPreprocessor::new();
            if let Err(e) = check_mdbook_version(preprocessor.name()) {
                eprintln!("Error: {e}");
                process::exit(1);
            }
            if let Err(e) = handle_preprocessing(&preprocessor) {
                eprintln!("Error running {} preprocessor: {}", preprocessor.name(), e);
                process::exit(1);
            }
        }
    }
}

fn handle_preprocessing(pre: &dyn Preprocessor) -> Result<(), Error> {
    let (ctx, book) = CmdPreprocessor::parse_input(io::stdin())?;
    let processed_book = pre.run(&ctx, book)?;
    serde_json::to_writer(io::stdout(), &processed_book)?;
    Ok(())
}

fn check_mdbook_version(preprocessor_name: &str) -> Result<(), Error> {
    let mdbook_version = Version::parse(mdbook::MDBOOK_VERSION)?;
    let version_req = VersionReq::parse(">=0.4.0")?;

    if !version_req.matches(&mdbook_version) {
        eprintln!(
            "Warning: The {} preprocessor was built against mdbook version req {}, \
             but we are running with version {}. If you encounter problems, \
             try installing a compatible version of the preprocessor.",
            preprocessor_name, version_req, mdbook_version
        );
    }
    Ok(())
}
