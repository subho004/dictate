use std::env;

#[derive(Clone)]
pub struct Config {
    pub assemblyai_api_key: String,
    pub gemini_api_key: String,
}

impl Config {
    /// Loads API keys from the environment. In dev, `.env` (project root, two levels up
    /// from src-tauri/target/.. at runtime we just search upward) is loaded via dotenvy.
    /// TODO(milestone 7 security pass): move these to the OS keychain via the `keyring`
    /// crate instead of env vars once the settings UI exists.
    pub fn load() -> anyhow::Result<Self> {
        // Try loading .env from cwd and a couple of parent dirs (dev convenience only).
        let _ = dotenvy::dotenv();
        for ancestor in ["..", "../..", "../../.."] {
            let _ = dotenvy::from_filename(format!("{ancestor}/.env"));
        }

        let assemblyai_api_key = env::var("ASSEMBLYAI_API_KEY")
            .map_err(|_| anyhow::anyhow!("ASSEMBLYAI_API_KEY not set (check .env)"))?;
        let gemini_api_key = env::var("GEMINI_API_KEY")
            .map_err(|_| anyhow::anyhow!("GEMINI_API_KEY not set (check .env)"))?;

        Ok(Self {
            assemblyai_api_key,
            gemini_api_key,
        })
    }
}
