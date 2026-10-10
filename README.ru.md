# PhotoCraft RU

Независимая русская сборка [PhotoCraft](https://github.com/storytold/photocraft), которую поддерживает **coolmorimer**. Основана на работе команды ArtCraft; не является её официальным выпуском.

Русский интерфейс, локальное удаление фона с сохранением маски и генеративная заливка отдельным слоем через API выбранного пользователем провайдера. Только генерация отправляет область изображения во внешний сервис. Модель удаления фона устанавливается по запросу.

Инструкция: [установка и ИИ-инструменты](docs/ru/INSTALL.txt). Установщик Windows и переносимая версия публикуются в [выпусках](https://github.com/coolmorimer/photocraft/releases).

## Сборка

Нужен Rust, указанный в `Cargo.toml`, и системные зависимости исходного проекта.

```powershell
$env:PHOTOCRAFT_DEFAULT_LANGUAGE = 'ru'
$env:PHOTOCRAFT_CONFIG_NAME = 'PhotoCraft-RU'
cargo build --release -p photocraft -p photocraft-cli
& 'C:\Program Files (x86)\Inno Setup 6\ISCC.exe' packaging/windows/installer-ru.iss
```

Поддержаны протоколы OpenAI Images Edits, OpenRouter Images и Gemini. Адрес, модель и API-ключ задаёт пользователь. Ключ хранится в памяти или защищённом хранилище ОС. Скорость, качество и оплата генерации зависят от провайдера. Совместимость с произвольными нестандартными API не гарантируется.

Редактор распространяется под MIT OR Apache-2.0. Модель удаления фона сохраняет собственную лицензию; см. инструкцию и `ATTRIBUTION.md`.
