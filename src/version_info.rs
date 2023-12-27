const CI_TAG: &str = env!("PVIEW_CI_TAG");
const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn pview_version() -> &'static str {
    if CI_TAG.is_empty() {
        PKG_VERSION
    } else {
        CI_TAG
    }
}
