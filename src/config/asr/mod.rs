pub mod apple;
pub mod doubao;
pub mod instance;
pub mod options;
pub mod tencent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalVadMode {
    Auto,
    On,
    Off,
}
