//! Secret-bearing settings are kept outside serializable app and automation state.
use photocraft_ml::{ApiConfig, Protocol};
use std::cell::RefCell;

#[derive(Default)]
struct Draft {
    open: bool,
    config: ApiConfig,
    key: String,
    remember: bool,
    status: String,
}
thread_local! { static DRAFT: RefCell<Draft> = RefCell::new(Draft::default()); }
pub fn open(ctx: &egui::Context) {
    DRAFT.with(|state| {
        let mut state = state.borrow_mut();
        state.config = photocraft_ml::api::config();
        state.key.clear();
        state.status.clear();
        state.open = true;
    });
    ctx.request_repaint();
}
pub fn show(ctx: &egui::Context) {
    DRAFT.with(|state| {
        let mut draft = state.borrow_mut();
        if !draft.open { return; }
        let mut open = true;
        let mut close = false;
        egui::Window::new("API генерации").id(egui::Id::new("generation_api_settings")).open(&mut open).default_width(540.0).resizable(true).show(ctx, |ui| {
            ui.label("Только генерация использует API. Удаление фона и остальные инструменты работают локально.");
            ui.add_space(8.0);
            let old = draft.config.protocol;
            egui::ComboBox::from_label("Сервис и протокол").selected_text(match old { Protocol::OpenAi => "OpenAI / совместимый API", Protocol::OpenRouter => "OpenRouter", Protocol::Gemini => "Google Gemini" }).show_ui(ui, |ui| {
                ui.selectable_value(&mut draft.config.protocol, Protocol::OpenAi, "OpenAI / совместимый API");
                ui.selectable_value(&mut draft.config.protocol, Protocol::OpenRouter, "OpenRouter");
                ui.selectable_value(&mut draft.config.protocol, Protocol::Gemini, "Google Gemini");
            });
            if old != draft.config.protocol { draft.config = ApiConfig::preset(draft.config.protocol); draft.key.clear(); }
            ui.label("Полный адрес запроса API");
            ui.add(egui::TextEdit::singleline(&mut draft.config.endpoint).desired_width(f32::INFINITY));
            ui.label("Название модели с поддержкой редактирования изображений");
            ui.add(egui::TextEdit::singleline(&mut draft.config.model).desired_width(f32::INFINITY));
            ui.label("API-ключ (пустое поле не заменяет уже сохранённый ключ)");
            ui.add(egui::TextEdit::singleline(&mut draft.key).password(true).desired_width(f32::INFINITY));
            ui.checkbox(&mut draft.remember, "Сохранить ключ в защищённом хранилище ОС");
            ui.small("Без сохранения ключ используется до закрытия редактора. Оплата и лимиты определяются вашим провайдером. Отмена запроса не гарантирует отмену списания у сервиса.");
            if !draft.status.is_empty() { ui.label(&draft.status); }
            ui.horizontal(|ui| {
                if ui.button("Применить").clicked() {
                    match photocraft_ml::api::configure(draft.config.clone(), draft.key.clone(), draft.remember) {
                        Ok(()) => { draft.key.clear(); close = true; }, Err(error) => draft.status = error,
                    }
                }
                if ui.button("Удалить сохранённый ключ").clicked() {
                    draft.key.clear();
                    draft.status = match photocraft_ml::api::forget() { Ok(()) => "Ключ удалён".into(), Err(e) => e };
                }
                if ui.button("Закрыть").clicked() { close = true; }
            });
        });
        if !open || close { draft.open = false; draft.key.clear(); }
    });
}
