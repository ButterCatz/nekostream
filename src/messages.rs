use serde::Deserialize;
use std::collections::HashMap;
use std::fs;

#[derive(Debug, Deserialize)]
struct TemplateFile {
    #[serde(flatten)]
    locales: HashMap<String, HashMap<String, String>>,
}

#[derive(Clone)]
pub struct MessageCatalog {
    locale: String,
    locales: HashMap<String, HashMap<String, String>>,
}

impl MessageCatalog {
    pub fn load(path: &str, locale: &str) -> Self {
        let parsed = fs::read_to_string(path)
            .ok()
            .and_then(|content| toml::from_str::<TemplateFile>(&content).ok())
            .map(|file| file.locales)
            .filter(|locales| !locales.is_empty());

        let locales = parsed.unwrap_or_else(default_locales);

        Self {
            locale: normalize_locale(locale),
            locales,
        }
    }

    pub fn t(&self, key: &str) -> String {
        self.locale_map()
            .and_then(|m| m.get(key))
            .or_else(|| self.locales.get("en-US").and_then(|m| m.get(key)))
            .cloned()
            .unwrap_or_else(|| key.to_string())
    }

    pub fn tr(&self, key: &str, vars: &[(&str, &str)]) -> String {
        let mut message = self.t(key);
        for (name, value) in vars {
            message = message.replace(&format!("{{{name}}}"), value);
        }
        message
    }

    pub fn locale(&self) -> &str {
        &self.locale
    }

    fn locale_map(&self) -> Option<&HashMap<String, String>> {
        self.locales.get(&self.locale)
    }
}

fn normalize_locale(locale: &str) -> String {
    match locale.to_ascii_lowercase().as_str() {
        "zh-tw" | "zh_tw" | "zh-hant" => "zh-TW".to_string(),
        "en" | "en-us" | "en_us" => "en-US".to_string(),
        _ => locale.to_string(),
    }
}

fn default_locales() -> HashMap<String, HashMap<String, String>> {
    let mut zh_tw = HashMap::new();
    zh_tw.insert("unknown_command".to_string(), "未知指令。".to_string());
    zh_tw.insert(
        "command_error".to_string(),
        "執行失敗：{error}".to_string(),
    );
    zh_tw.insert(
        "join_already_in_channel".to_string(),
        "已在你的語音頻道。".to_string(),
    );
    zh_tw.insert("join_success".to_string(), "已加入語音頻道。".to_string());
    zh_tw.insert("leave_success".to_string(), "已離開語音頻道。".to_string());
    zh_tw.insert(
        "start_already_active".to_string(),
        "此伺服器已在串流中。".to_string(),
    );
    zh_tw.insert(
        "start_enabled".to_string(),
        "已開始串流。".to_string(),
    );
    zh_tw.insert(
        "stop_success".to_string(),
        "已停止串流。".to_string(),
    );
    zh_tw.insert(
        "stop_already_stopped".to_string(),
        "目前沒有進行中的串流。".to_string(),
    );
    zh_tw.insert(
        "autoleave_set_disabled".to_string(),
        "已關閉自動離開。".to_string(),
    );
    zh_tw.insert(
        "autoleave_set_when_no_humans".to_string(),
        "已啟用自動離開（無真人時離開）。".to_string(),
    );
    zh_tw.insert(
        "autoleave_status".to_string(),
        "目前自動離開模式：{mode}".to_string(),
    );
    zh_tw.insert("autoleave_mode_disabled".to_string(), "關閉".to_string());
    zh_tw.insert(
        "autoleave_mode_when_no_humans".to_string(),
        "無真人即離開".to_string(),
    );
    zh_tw.insert(
        "status_summary".to_string(),
        "串流：{streaming}\n自動離開：{autoleave}".to_string(),
    );
    zh_tw.insert(
        "status_streaming_active".to_string(),
        "進行中".to_string(),
    );
    zh_tw.insert(
        "status_streaming_inactive".to_string(),
        "未開始".to_string(),
    );

    let mut en_us = HashMap::new();
    en_us.insert("unknown_command".to_string(), "Unknown command.".to_string());
    en_us.insert(
        "command_error".to_string(),
        "Command failed: {error}".to_string(),
    );
    en_us.insert(
        "join_already_in_channel".to_string(),
        "Already in your voice channel.".to_string(),
    );
    en_us.insert("join_success".to_string(), "Joined voice channel.".to_string());
    en_us.insert("leave_success".to_string(), "Left voice channel.".to_string());
    en_us.insert(
        "start_already_active".to_string(),
        "Streaming is already active for this server.".to_string(),
    );
    en_us.insert("start_enabled".to_string(), "Streaming started.".to_string());
    en_us.insert("stop_success".to_string(), "Streaming stopped.".to_string());
    en_us.insert(
        "stop_already_stopped".to_string(),
        "No active stream right now.".to_string(),
    );
    en_us.insert(
        "autoleave_set_disabled".to_string(),
        "Autoleave disabled.".to_string(),
    );
    en_us.insert(
        "autoleave_set_when_no_humans".to_string(),
        "Autoleave enabled (leave when no humans remain).".to_string(),
    );
    en_us.insert(
        "autoleave_status".to_string(),
        "Current autoleave mode: {mode}".to_string(),
    );
    en_us.insert(
        "autoleave_mode_disabled".to_string(),
        "Disabled".to_string(),
    );
    en_us.insert(
        "autoleave_mode_when_no_humans".to_string(),
        "Leave when no humans remain".to_string(),
    );
    en_us.insert(
        "status_summary".to_string(),
        "Streaming: {streaming}\nAutoleave: {autoleave}".to_string(),
    );
    en_us.insert(
        "status_streaming_active".to_string(),
        "Active".to_string(),
    );
    en_us.insert(
        "status_streaming_inactive".to_string(),
        "Inactive".to_string(),
    );

    let mut locales = HashMap::new();
    locales.insert("zh-TW".to_string(), zh_tw);
    locales.insert("en-US".to_string(), en_us);
    locales
}
