//! Pure templates and send-time absolute scheduling. No scheduler is called here.
use chrono::{Local, TimeZone};
use serde::Serialize;

pub const MAX_MESSAGE_LENGTH: usize = 2000;

pub fn valid_message(text: &str) -> bool {
    !text.trim().is_empty() && text.encode_utf16().count() <= MAX_MESSAGE_LENGTH
}

pub fn default_pause(language: &str) -> String {
    crate::pause_control::pause_message_for_language(language).to_owned()
}

/// The saved text, or the language's plain pause when it was never filled in
/// (unmigrated settings), so delivery never sends an empty or invalid prompt.
pub fn effective_message(text: &str, language: &str) -> String {
    if valid_message(text) { text.to_owned() } else { default_pause(language) }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Templates { pub pause: String, pub continuation: String }

pub fn templates(language: &str) -> Templates {
    let continuation = match crate::commands::resolve_locale(language) {
        "zh-CN" => "暂停当前工作。请在当前聊天安排一次性继续唤醒，指定时间为 {resume_at}。直接使用此时间，不再加任何延迟。\n\n先检查本聊天是否已有同一用途、同一目标时间的继续任务；有则核验并复用，不重复创建。使用当前可用的 automation_update 工具，mode=create、kind=heartbeat、destination=thread、name=继续、status=ACTIVE。\n\n保存的任务 prompt 必须包含：这是一次性继续唤醒。先从心跳上下文取得本任务 automation_id，读取并暂停该任务，保留其他配置；确认已停用且没有下次执行时间后，再继续原工作。停用失败则报告错误并停止，不重新创建任务。\n\n核对指定时间仍在未来，确认调度器时区准确表示此绝对时刻。按当前工具 schema 生成单次规则；若使用每日频率和单次计数，须指定本地小时、分钟和秒，直接创建不传 DTSTART。不得重新计算剩余时长、再次加两分钟或自行顺延到次日。创建后保存真实任务 ID，核对聊天、日期、时区和下次执行时间；工具不可用、时间无法确认或创建失败则报告原因并停止，不反复创建。成功后简短确认并结束回复，等待唤醒。",
        "zh-Hant" => "暫停目前工作。請在目前聊天安排一次性繼續喚醒，指定時間為 {resume_at}。直接使用此時間，不再加任何延遲。\n\n先檢查本聊天是否已有同用途、同目標時間的任務；有則核驗並重用，不重複建立。使用可用的 automation_update，mode=create、kind=heartbeat、destination=thread、name=繼續、status=ACTIVE。\n\n儲存的任務 prompt 必須包含：這是一次性繼續喚醒。先從心跳上下文取得本任務 automation_id，讀取並暫停該任務，保留其他設定；確認已停用且沒有下次執行時間後，再繼續原工作。停用失敗則報告錯誤並停止，不重新建立任務。\n\n核對指定時間仍在未來，確認排程器時區準確表示此絕對時刻。按目前工具 schema 產生單次規則；若使用每日頻率與單次計數，指定本地時、分、秒，直接建立不傳 DTSTART。不得再次加兩分鐘或自行延至次日。建立後儲存真實任務 ID，核對聊天、日期、時區和下次執行時間；工具不可用、時間無法確認或建立失敗則報告原因並停止，不反覆建立。成功後結束回覆，等待喚醒。",
        _ => "Pause the current work. Schedule a one-time continuation in this chat at exactly {resume_at}. Use this absolute time without adding any delay.\n\nCheck for an existing task for this chat, purpose and target time; verify and reuse it rather than creating a duplicate. Use the available automation_update tool: mode=create, kind=heartbeat, destination=thread, name=Continue, status=ACTIVE.\n\nThe saved task prompt must say: This is a one-time continuation. First obtain this task's automation_id from the heartbeat context, read and pause that task while preserving its other settings, and verify that it is paused with no next run. Only then continue the original work. If pausing fails, report the error and stop; do not recreate the task.\n\nVerify the target is still in the future and the scheduler time zone represents that instant. Follow the current tool schema. If using daily frequency with a single count, specify local hour, minute and second; omit DTSTART on direct creation. Do not add another two minutes or roll an expired target into tomorrow. Save the real task ID and verify chat, date, zone and next run. If unavailable, unverifiable or failed, report and stop without repeated creates. On success, briefly confirm and end this reply to await the wake-up.",
    };
    Templates { pause: default_pause(language), continuation: continuation.into() }
}

/// The injected formatter makes overflow/expiry/zone failures deterministic in tests.
pub fn render_with(template: &str, reset: Option<i64>, now: i64, fallback: &str,
    format: impl FnOnce(i64) -> Option<String>) -> String {
    if !template.contains("{resume_at}") { return template.into(); }
    let target = reset.filter(|reset| *reset > now).and_then(|reset| reset.checked_add(120));
    target.and_then(format).map(|at| template.replace("{resume_at}", &at)).unwrap_or_else(|| fallback.into())
}

pub struct TimeZoneContext { zone:Option<String> }
impl TimeZoneContext {pub fn system()->Self {Self{zone:iana_time_zone::get_timezone().ok()}}}

pub fn render_five_hour(template: &str, reset: Option<i64>, now: i64, language: &str, context:&TimeZoneContext) -> String {
    render_with(template, reset, now, &default_pause(language), |target| {
        let zone=context.zone.as_ref()?;
        if iana_time_zone::get_timezone().ok().as_ref()!=Some(zone){return None;}
        let at=Local.timestamp_opt(target,0).single()?;
        if iana_time_zone::get_timezone().ok().as_ref()!=Some(zone){return None;}
        Some(format!("{} ({zone})",at.format("%Y-%m-%d %H:%M:%S %:z")))
    })
}

#[tauri::command]
pub fn get_pause_templates(language: String) -> Templates { templates(&language) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absolute_target_is_reset_plus_120_not_send_time_and_replaces_every_variable() {
        for now in [1000, 1040, 1099] {
            assert_eq!(render_with("{resume_at} / {resume_at}",Some(1100),now,"Pause",|at|Some(at.to_string())),"1220 / 1220");
        }
        assert_eq!(render_with("fixed",None,1000,"Pause",|_|panic!("must not format")),"fixed");
    }
    #[test]
    fn invalid_times_or_unknown_zone_fall_back_without_residual_placeholders() {
        for reset in [None,Some(0),Some(1000),Some(i64::MAX)] {
            assert_eq!(render_with("at {resume_at}",reset,1000,"Pause",|_|Some("bad".into())),"Pause");
        }
        assert_eq!(render_with("{resume_at}",Some(1100),1000,"Pause",|_|None),"Pause");
    }
    #[test]
    fn unicode_limits_and_all_localized_presets_share_the_time_and_cleanup_contract() {
        assert!(valid_message(&"😀".repeat(1000)));assert!(!valid_message(&"😀".repeat(1001)));assert!(!valid_message(" \n"));
        for locale in ["en-US","zh-CN","zh-Hant"] {
            let t=templates(locale);assert!(valid_message(&t.continuation));assert!(t.continuation.contains("{resume_at}"));assert!(t.continuation.contains("automation_id"));
        }
    }
}

#[cfg(test)]
mod calendar_tests {
    use super::*;
    #[test]
    fn exact_seconds_and_calendar_rollover_are_preserved() {
        let reset=chrono::DateTime::parse_from_rfc3339("2026-12-31T23:59:20-05:00").unwrap().timestamp();
        let result=render_with("at {resume_at}",Some(reset),reset-61,"Pause",|target| {
            Some(chrono::FixedOffset::west_opt(18000).unwrap().timestamp_opt(target,0).single().unwrap().format("%Y-%m-%d %H:%M:%S %:z").to_string())
        });
        assert_eq!(result,"at 2027-01-01 00:01:20 -05:00");
        let invalid=TimeZoneContext{zone:None};assert_eq!(render_five_hour("{resume_at}",Some(reset),reset-61,"en-US",&invalid),"Pause");
    }
}
