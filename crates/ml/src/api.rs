//! User-selected image APIs. Credentials never enter command parameters or document data.
use crate::Result;
use base64::Engine;
use image::ImageEncoder;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Protocol {
    #[default]
    OpenAi,
    OpenRouter,
    Gemini,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApiConfig {
    pub protocol: Protocol,
    /// Full request URL. Gemini uses {model} as a placeholder.
    pub endpoint: String,
    pub model: String,
}
impl Default for ApiConfig {
    fn default() -> Self {
        Self::preset(Protocol::OpenAi)
    }
}
impl ApiConfig {
    pub fn preset(protocol: Protocol) -> Self {
        let (endpoint, model) = match protocol {
            Protocol::OpenAi => ("https://api.openai.com/v1/images/edits", "gpt-image-1"),
            Protocol::OpenRouter => ("https://openrouter.ai/api/v1/images", "google/gemini-2.5-flash-image"),
            Protocol::Gemini => ("https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent", "gemini-2.5-flash-image"),
        };
        Self { protocol, endpoint: endpoint.into(), model: model.into() }
    }
    pub fn validate(&self) -> Result<reqwest::Url> {
        if self.model.is_empty() || self.model.len() > 256 || self.model.chars().any(char::is_control) {
            return Err("Укажите корректное название модели".into());
        }
        let url = reqwest::Url::parse(&self.endpoint.replace("{model}", &self.model)).map_err(|_| "Некорректный адрес API")?;
        let local = matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "::1"));
        if !(url.scheme() == "https" || url.scheme() == "http" && local)
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("Для API нужен HTTPS-адрес без ключа в URL; HTTP разрешён только для локального сервера".into());
        }
        Ok(url)
    }
    fn credential(&self) -> Result<keyring::Entry> {
        let id = format!("{:x}", Sha256::digest(format!("{:?}\n{}\n{}", self.protocol, self.endpoint, self.model)));
        keyring::Entry::new("coolmorimer.PhotoCraft-RU.image-api", &id).map_err(|_| "Хранилище ключей ОС недоступно".into())
    }
}

// Session-only fallback. It is intentionally not Serialize or Debug.
static ACTIVE: Mutex<Option<(ApiConfig, String)>> = Mutex::new(None);
fn config_path() -> Result<std::path::PathBuf> {
    let dirs = directories::ProjectDirs::from("io", "coolmorimer", "PhotoCraft-RU").ok_or("Не найдена папка настроек")?;
    Ok(dirs.config_dir().join("generation-api.json"))
}
pub fn config() -> ApiConfig {
    if let Some((config, _)) = ACTIVE.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
        return config.clone();
    }
    config_path()
        .ok()
        .filter(|p| std::fs::metadata(p).is_ok_and(|m| m.len() < 16 * 1024))
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
pub fn configure(config: ApiConfig, key: String, remember: bool) -> Result<()> {
    config.validate()?;
    let key = if key.trim().is_empty() { credentials(&config)? } else { key };
    if key.trim().is_empty() || key.len() > 8192 || key.chars().any(char::is_control) {
        return Err("Введите API-ключ".into());
    }
    if remember {
        config
            .credential()?
            .set_password(&key)
            .map_err(|_| "Не удалось сохранить ключ в хранилище ОС. Снимите флажок сохранения для работы до закрытия редактора.")?;
    }
    let path = config_path()?;
    std::fs::create_dir_all(path.parent().ok_or("Некорректная папка настроек")?).map_err(|_| "Не удалось создать папку настроек")?;
    std::fs::write(path, serde_json::to_vec_pretty(&config).map_err(|_| "Не удалось записать настройки")?).map_err(|_| "Не удалось записать настройки")?;
    *ACTIVE.lock().unwrap_or_else(PoisonError::into_inner) = Some((config, key));
    Ok(())
}
pub fn forget() -> Result<()> {
    let cfg = config();
    *ACTIVE.lock().unwrap_or_else(PoisonError::into_inner) = None;
    match cfg.credential()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err("Не удалось удалить сохранённый ключ".into()),
    }
}
fn credentials(cfg: &ApiConfig) -> Result<String> {
    if let Some((active, key)) = ACTIVE.lock().unwrap_or_else(PoisonError::into_inner).as_ref().filter(|(c, _)| c == cfg) {
        let _ = active;
        return Ok(key.clone());
    }
    cfg.credential()?.get_password().map_err(|_| "Откройте «Настроить API генерации» и введите ключ".into())
}
fn png(bytes: &[u8], w: usize, h: usize, color: image::ExtendedColorType) -> Result<Vec<u8>> {
    let mut result = Vec::new();
    image::codecs::png::PngEncoder::new(&mut result).write_image(bytes, w as u32, h as u32, color).map_err(|_| "Ошибка подготовки PNG")?;
    Ok(result)
}
fn data_url(bytes: &[u8]) -> String {
    format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))
}
const RESPONSE_LIMIT: usize = 48 * 1024 * 1024;
async fn bounded(mut response: reqwest::Response) -> Result<Vec<u8>> {
    if response.content_length().is_some_and(|n| n > RESPONSE_LIMIT as u64) {
        return Err("Слишком большой ответ API".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "Не удалось прочитать ответ API")? {
        if bytes.len().saturating_add(chunk.len()) > RESPONSE_LIMIT {
            return Err("Слишком большой ответ API".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
fn image_value(protocol: Protocol, value: &Value) -> Result<&str> {
    let result = match protocol {
        Protocol::OpenAi | Protocol::OpenRouter => value.pointer("/data/0/b64_json").and_then(Value::as_str),
        Protocol::Gemini => value
            .pointer("/candidates/0/content/parts")
            .and_then(Value::as_array)
            .and_then(|parts| parts.iter().find_map(|p| p.pointer("/inlineData/data").or_else(|| p.pointer("/inline_data/data")).and_then(Value::as_str))),
    };
    result.ok_or_else(|| {
        "API не вернул изображение. Проверьте, поддерживает ли модель редактирование изображений; запрос также мог быть отклонён сервисом.".into()
    })
}
fn decode(encoded: &str, width: usize, height: usize) -> Result<Vec<u8>> {
    if encoded.len() > RESPONSE_LIMIT {
        return Err("Слишком большое изображение API".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).map_err(|_| "Некорректное изображение API")?;
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().map_err(|_| "Неизвестный формат ответа API")?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(96 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().map_err(|_| "Не удалось прочитать изображение API")?.to_rgb8();
    Ok(image::imageops::resize(&img, width as u32, height as u32, image::imageops::FilterType::Triangle).into_raw())
}

/// Only a crop and mask supplied by the generation command are transmitted.
/// Cancelling stops the local request; the provider may still bill already-started work.
pub fn generate(prompt: &str, width: usize, height: usize, rgb: &[u8], mask: &[u8], cancelled: impl Fn() -> bool) -> Result<Vec<u8>> {
    crate::validate_image(width, height, rgb.len())?;
    if width > 1024 || height > 1024 || mask.len() != width * height || prompt.trim().is_empty() || prompt.len() > 4096 {
        return Err("Некорректные параметры генерации".into());
    }
    let _guard = crate::native::WORKER.try_lock().map_err(|_| "ИИ-операция уже выполняется")?;
    let cfg = config();
    let key = credentials(&cfg)?;
    request(&cfg, &key, prompt, width, height, rgb, mask, cancelled)
}

#[allow(clippy::too_many_arguments)]
fn request(cfg: &ApiConfig, key: &str, prompt: &str, width: usize, height: usize, rgb: &[u8], mask: &[u8], cancelled: impl Fn() -> bool) -> Result<Vec<u8>> {
    let url = cfg.validate()?;
    if cancelled() {
        return Err("Операция отменена".into());
    }
    let source = png(rgb, width, height, image::ExtendedColorType::Rgb8)?;
    let mask_image = png(mask, width, height, image::ExtendedColorType::L8)?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|_| "Ошибка HTTP-клиента")?;
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|_| "Не удалось запустить запрос API")?;
    runtime.block_on(async {
        let operation = async {
            let edit_prompt = format!("Edit the first image. The second image is a selection mask: white means edit, black means preserve. Preserve composition, framing and all unselected pixels. Return only the edited image. User request: {prompt}");
            let mut request = client.post(url);
            request = match cfg.protocol {
                Protocol::OpenAi => {
                    let rgba: Vec<u8> = mask.iter().flat_map(|v| [255,255,255,255-v]).collect();
                    let alpha = png(&rgba, width, height, image::ExtendedColorType::Rgba8)?;
                    let image = reqwest::multipart::Part::bytes(source).file_name("image.png").mime_str("image/png").map_err(|_| "Ошибка PNG")?;
                    let mask = reqwest::multipart::Part::bytes(alpha).file_name("mask.png").mime_str("image/png").map_err(|_| "Ошибка маски")?;
                    request.bearer_auth(key).multipart(reqwest::multipart::Form::new().text("model", cfg.model.clone()).text("prompt", prompt.to_owned()).text("n", "1").part("image", image).part("mask", mask))
                }
                Protocol::OpenRouter => request.bearer_auth(key).json(&json!({"model":cfg.model,"prompt":edit_prompt,"n":1,"output_format":"png","input_references":[{"type":"image_url","image_url":{"url":data_url(&source)}},{"type":"image_url","image_url":{"url":data_url(&mask_image)}}]})),
                Protocol::Gemini => request.header("x-goog-api-key", key).json(&json!({"contents":[{"parts":[{"text":edit_prompt},{"inline_data":{"mime_type":"image/png","data":base64::engine::general_purpose::STANDARD.encode(&source)}},{"inline_data":{"mime_type":"image/png","data":base64::engine::general_purpose::STANDARD.encode(&mask_image)}}]}],"generationConfig":{"responseModalities":["TEXT","IMAGE"]}})),
            };
            // Never expose raw transport errors, provider bodies, URLs or headers containing credentials.
            let response = request.send().await.map_err(|_| "Не удалось связаться с API. Проверьте адрес и подключение")?;
            if !response.status().is_success() {
                return Err(match response.status().as_u16() {
                    401 | 403 => "API отклонил ключ или доступ к модели",
                    429 => "Лимит API исчерпан. Проверьте баланс и ограничения провайдера",
                    400 | 404 | 422 => "API не принял запрос. Проверьте адрес, модель и поддержку редактирования изображений",
                    _ => "Ошибка сервиса генерации. Повторите запрос позднее",
                }.into());
            }
            let value: Value = serde_json::from_slice(&bounded(response).await?).map_err(|_| "Некорректный JSON в ответе API")?;
            decode(image_value(cfg.protocol, &value)?, width, height)
        };
        let cancellation = async { loop { if cancelled() { break; } tokio::time::sleep(Duration::from_millis(100)).await; } };
        tokio::select! { result = operation => result, () = cancellation => Err("Операция отменена".into()) }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    #[test]
    fn provider_errors_are_redacted_and_pending_requests_can_be_cancelled() {
        for cancelled in [false, true] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let cfg = ApiConfig { endpoint: format!("http://{}/images", listener.local_addr().unwrap()), ..Default::default() };
            let worker = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                let mut buffer = [0; 8192];
                let _ = stream.read(&mut buffer);
                if cancelled { std::thread::sleep(Duration::from_millis(350)); }
                else { let _ = write!(stream, "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 17\r\nConnection: close\r\n\r\nsecret-test-token"); }
            });
            let start = std::time::Instant::now();
            let error = request(&cfg, "secret-test-token", "flower", 8, 8, &[42; 192], &[255; 64], || cancelled && start.elapsed() > Duration::from_millis(150)).unwrap_err();
            assert!(!error.contains("secret-test-token"));
            if cancelled { assert!(error.contains("отменена")); } else { assert!(error.contains("Лимит")); }
            worker.join().unwrap();
        }
    }
    #[test]
    fn all_protocols_send_expected_requests_and_accept_real_png_responses() {
        for protocol in [Protocol::OpenAi, Protocol::OpenRouter, Protocol::Gemini] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let cfg = ApiConfig { protocol, endpoint: format!("http://{}/images", listener.local_addr().unwrap()), model: "test-image-model".into() };
            let b64 = base64::engine::general_purpose::STANDARD.encode(png(&[123; 192], 8, 8, image::ExtendedColorType::Rgb8).unwrap());
            let worker = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0; 4096];
                loop {
                    let n = stream.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                        let length: usize = headers.lines().find_map(|l| l.strip_prefix("content-length:")).unwrap().trim().parse().unwrap();
                        if bytes.len() < end + 4 + length {
                            continue;
                        }
                        assert!(headers.starts_with("post /images "));
                        assert!(headers.contains(if protocol == Protocol::Gemini { "x-goog-api-key: test-key" } else { "authorization: bearer test-key" }));
                        if protocol == Protocol::OpenAi {
                            let body = String::from_utf8_lossy(&bytes[end + 4..]);
                            assert!(body.contains("test-image-model"));
                            assert!(body.contains("filename=\"mask.png\""));
                        } else {
                            let body: Value = serde_json::from_slice(&bytes[end + 4..]).unwrap();
                            if protocol == Protocol::OpenRouter {
                                assert_eq!(body["input_references"].as_array().unwrap().len(), 2);
                            } else {
                                assert_eq!(body["contents"][0]["parts"].as_array().unwrap().len(), 3);
                            }
                        }
                        break;
                    }
                }
                let value = if protocol == Protocol::Gemini {
                    json!({"candidates":[{"content":{"parts":[{"inlineData":{"data":b64}}]}}]})
                } else {
                    json!({"data":[{"b64_json":b64}]})
                };
                let body = value.to_string();
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                    .unwrap();
            });
            assert_eq!(request(&cfg, "test-key", "a flower", 8, 8, &[25; 192], &[255; 64], || false).unwrap(), vec![123; 192]);
            worker.join().unwrap();
        }
    }
    #[test]
    fn rejects_unsafe_endpoints_and_preserves_custom_models() {
        let mut c = ApiConfig::default();
        assert!(c.validate().is_ok());
        for url in ["http://example.com/api", "https://key@example.com/api", "https://example.com/api?key=secret", "file:///tmp/image"] {
            c.endpoint = url.into();
            assert!(c.validate().is_err());
        }
        c.endpoint = "http://127.0.0.1:18283/test".into();
        c.model = "my-custom-model".into();
        assert!(c.validate().is_ok());
    }
    #[test]
    fn decodes_provider_response_formats_and_bounds_images() {
        let b64 = base64::engine::general_purpose::STANDARD.encode(png(&[123; 192], 8, 8, image::ExtendedColorType::Rgb8).unwrap());
        for protocol in [Protocol::OpenAi, Protocol::OpenRouter, Protocol::Gemini] {
            let value = if protocol == Protocol::Gemini {
                json!({"candidates":[{"content":{"parts":[{"text":"done"},{"inlineData":{"data":b64}}]}}]})
            } else {
                json!({"data":[{"b64_json":b64}]})
            };
            assert_eq!(decode(image_value(protocol, &value).unwrap(), 8, 8).unwrap(), vec![123; 192]);
        }
        assert!(decode("invalid", 8, 8).is_err());
        assert!(image_value(Protocol::OpenAi, &json!({"error":"rejected"})).is_err());
    }
}
