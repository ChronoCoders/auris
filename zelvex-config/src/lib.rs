use std::{net::SocketAddr, path::Path, path::PathBuf};

use alloy::primitives::Address;
use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed reading config file: {0}")]
    Read(#[from] std::io::Error),
    #[error("failed parsing TOML: {0}")]
    ParseToml(#[from] toml::de::Error),
    #[error("invalid env override {key}: {value}")]
    InvalidEnvOverride { key: String, value: String },
}

#[derive(Debug, Deserialize)]
pub struct Config {
    pub node: NodeConfig,
    pub flashbots: FlashbotsConfig,
    pub bot: BotConfig,
    pub pools: PoolsConfig,
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub auth: AuthConfig,
    pub keys: KeysConfig,
}

#[derive(Debug, Deserialize)]
pub struct NodeConfig {
    pub ws_url: String,
}

#[derive(Debug, Deserialize)]
pub struct FlashbotsConfig {
    pub relay_url: String,
}

#[derive(Debug, Deserialize)]
pub struct BotConfig {
    pub min_profit_usd: f64,
    pub max_gas_gwei: u64,
    pub paper_trade: bool,
}

#[derive(Debug, Deserialize)]
pub struct PoolsConfig {
    #[serde(default)]
    pub seed_pairs: Vec<String>,
    #[serde(default = "default_min_liquidity_usd")]
    pub min_liquidity_usd: f64,
}

fn default_min_liquidity_usd() -> f64 {
    500_000.0
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    pub bind_addr: SocketAddr,
    pub web_ui_path: PathBuf,
}

#[derive(Debug, Deserialize)]
pub struct DatabaseConfig {
    pub path: PathBuf,
}

#[derive(Debug, Deserialize)]
pub struct AuthConfig {
    pub jwt_secret: String,
    #[serde(default = "default_jwt_expiry")]
    pub jwt_expiry_seconds: u64,
}

fn default_jwt_expiry() -> u64 {
    86400
}

#[derive(Debug, Deserialize)]
pub struct KeysConfig {
    pub signer_key_path: PathBuf,
    pub flashbots_key_path: PathBuf,
    #[serde(deserialize_with = "deserialize_address")]
    pub contract_address: Address,
    #[serde(deserialize_with = "deserialize_address")]
    pub aave_pool_provider: Address,
}

fn deserialize_address<'de, D>(deserializer: D) -> Result<Address, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    s.parse::<Address>()
        .map_err(|_| serde::de::Error::custom("invalid address"))
}

pub fn load(path: &Path) -> Result<Config, ConfigError> {
    let _ = dotenvy::dotenv();

    let content = std::fs::read_to_string(path)?;
    let mut config: Config = toml::from_str(&content)?;

    apply_env_overrides(&mut config)?;

    Ok(config)
}

fn apply_env_overrides(config: &mut Config) -> Result<(), ConfigError> {
    env_override(&mut config.node.ws_url, "ZELVEX_NODE_WS_URL", |v| {
        Ok(v.to_string())
    })?;
    env_override(
        &mut config.flashbots.relay_url,
        "ZELVEX_FLASHBOTS_RELAY_URL",
        |v| Ok(v.to_string()),
    )?;
    env_override(
        &mut config.bot.min_profit_usd,
        "ZELVEX_BOT_MIN_PROFIT_USD",
        |v| v.parse::<f64>().map_err(|_| ()),
    )?;
    env_override(
        &mut config.bot.max_gas_gwei,
        "ZELVEX_BOT_MAX_GAS_GWEI",
        |v| v.parse::<u64>().map_err(|_| ()),
    )?;
    env_override(&mut config.bot.paper_trade, "ZELVEX_BOT_PAPER_TRADE", |v| {
        v.parse::<bool>().map_err(|_| ())
    })?;
    env_override(
        &mut config.pools.seed_pairs,
        "ZELVEX_POOLS_SEED_PAIRS",
        |v| {
            Ok(v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect())
        },
    )?;
    env_override(
        &mut config.pools.min_liquidity_usd,
        "ZELVEX_POOLS_MIN_LIQUIDITY_USD",
        |v| v.parse::<f64>().map_err(|_| ()),
    )?;
    env_override(
        &mut config.server.bind_addr,
        "ZELVEX_SERVER_BIND_ADDR",
        |v| v.parse::<SocketAddr>().map_err(|_| ()),
    )?;
    env_override(
        &mut config.server.web_ui_path,
        "ZELVEX_SERVER_WEB_UI_PATH",
        |v| Ok(PathBuf::from(v)),
    )?;
    env_override(&mut config.database.path, "ZELVEX_DATABASE_PATH", |v| {
        Ok(PathBuf::from(v))
    })?;
    env_override(&mut config.auth.jwt_secret, "ZELVEX_AUTH_JWT_SECRET", |v| {
        Ok(v.to_string())
    })?;
    env_override(
        &mut config.auth.jwt_expiry_seconds,
        "ZELVEX_AUTH_JWT_EXPIRY_SECONDS",
        |v| v.parse::<u64>().map_err(|_| ()),
    )?;
    env_override(
        &mut config.keys.signer_key_path,
        "ZELVEX_KEYS_SIGNER_KEY_PATH",
        |v| Ok(PathBuf::from(v)),
    )?;
    env_override(
        &mut config.keys.flashbots_key_path,
        "ZELVEX_KEYS_FLASHBOTS_KEY_PATH",
        |v| Ok(PathBuf::from(v)),
    )?;
    env_override(
        &mut config.keys.contract_address,
        "ZELVEX_KEYS_CONTRACT_ADDRESS",
        |v| v.parse::<Address>().map_err(|_| ()),
    )?;
    env_override(
        &mut config.keys.aave_pool_provider,
        "ZELVEX_KEYS_AAVE_POOL_PROVIDER",
        |v| v.parse::<Address>().map_err(|_| ()),
    )?;

    Ok(())
}

fn env_override<T, F>(target: &mut T, key: &str, parse: F) -> Result<(), ConfigError>
where
    F: FnOnce(&str) -> Result<T, ()>,
{
    let Ok(value) = std::env::var(key) else {
        return Ok(());
    };

    match parse(&value) {
        Ok(parsed) => {
            *target = parsed;
            Ok(())
        }
        Err(()) => Err(ConfigError::InvalidEnvOverride {
            key: key.to_string(),
            value,
        }),
    }
}
