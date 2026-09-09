use serde::Deserialize;
#[derive(Deserialize, Clone, Debug)]
pub struct Config {
    #[serde(alias = "headerName")]
    pub header_name: Option<String>,
    #[serde(alias = "maxDepth")]
    pub max_depth: Option<i64>,
    #[serde(alias = "maxFanout")]
    pub max_fanout: Option<i64>,
    #[serde(alias = "onExceed")]
    pub on_exceed: Option<String>,
    #[serde(alias = "windowMillis")]
    pub window_millis: Option<i64>,
}
#[pdk::hl::entrypoint_flex]
fn init(abi: &dyn pdk::flex_abi::api::FlexAbi) -> Result<(), anyhow::Error> {
    abi.setup()?;
    Ok(())
}
