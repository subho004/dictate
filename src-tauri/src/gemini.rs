//! Gemini 3.7-flash REST client.

use serde_json::json;

const MODEL: &str = "gemini-3.7-flash";

fn endpoint(api_key: &str) -> String {
    format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{MODEL}:generateContent?key={api_key}"
    )
}

/// The user had `selected_text` highlighted when they pressed the hotkey, then spoke
/// `instruction` — e.g. selected a paragraph and said "make this more formal". Returns
/// only the transformed replacement text, ready to paste directly over the selection.
///
/// `instruction_language` is the language AssemblyAI detected the spoken instruction
/// was in (e.g. "en", "es"), if known — purely informational context, since the
/// instruction being spoken in one language doesn't mean the output should switch to
/// it: TEXT's own language always governs unless INSTRUCTION explicitly asks for a
/// translation or language change.
pub async fn transform_selection(
    api_key: &str,
    selected_text: &str,
    instruction: &str,
    instruction_language: Option<&str>,
) -> anyhow::Result<String> {
    let client = reqwest::Client::new();

    let system = "You transform the given TEXT according to the user's spoken INSTRUCTION. \
         Preserve the original language TEXT is written in — do not translate or switch \
         languages, even if INSTRUCTION was spoken in a different language — unless \
         INSTRUCTION explicitly asks for a translation or language change. \
         Return ONLY the transformed text — no preamble, no explanation, no quotes \
         or markdown fencing around it, since it is pasted directly in place of the \
         original.";

    let user_text = match instruction_language {
        Some(lang) => {
            format!("INSTRUCTION (spoken in language \"{lang}\"): {instruction}\n\nTEXT:\n{selected_text}")
        }
        None => format!("INSTRUCTION: {instruction}\n\nTEXT:\n{selected_text}"),
    };

    let body = json!({
        "system_instruction": { "parts": [{ "text": system }] },
        "contents": [{ "role": "user", "parts": [{ "text": user_text }] }],
        "generationConfig": {
            "thinkingConfig": { "thinking_level": "low" }
        }
    });

    let resp = client
        .post(endpoint(api_key))
        .json(&body)
        .send()
        .await?
        .error_for_status()?;

    let parsed: serde_json::Value = resp.json().await?;
    let text = parsed["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("unexpected Gemini response shape: {parsed}"))?
        .to_string();

    Ok(text)
}
