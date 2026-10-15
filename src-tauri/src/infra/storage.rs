//! Выбор корней приложения: **единственное** место, где решается, куда писать.
//!
//! Вся механика — в [`ko_json_store::roots`]. Этот модуль отвечает только за
//! две вещи, которых крейт не знать может: превратить `AppHandle` и `%APPDATA%`
//! в кандидатов и один раз разнести данные из прежнего корня в новый.
//!
//! Метки:
//! - [`LABEL_APP_DATA`] — сессии, `app_config.json`, логи промптов. Перенос
//!   данных выполняется: потерять старые сессии при «починке» недопустимо.
//! - [`LABEL_APP_SHARED`] — `%APPDATA%/<плагин>` для всех плагинов репозитория.
//!   Общий для всех хостов на машине, поэтому и переносится.
//! - [`LABEL_SCRATCH`] — временные файлы процесса, включая `TMP`/`TEMP` для
//!   дочерних процессов. Переноса нет: временные файлы нечего переносить.
//!
//! ## Почему каталоги хоста и плагинов разнесены по разным подпапкам
//!
//! Запасной корень один и тот же — папка `data` рядом с exe. Но данные хоста
//! кладутся в `data/app`, а плагины — в `data/<плагин>`. Иначе проверка «перенос
//! уже был» (по метке-файлу) работала бы на общем каталоге: плагины переезжают
//! первыми, пишут метку, и хост решил бы, что его данные тоже уже перенесены.
//! Именно так 14.10 потерялись вкладки и история сессий.
//!
//! ## Почему `KingOrchData` здесь нет
//!
//! Крупное хранилище движков и моделей **не выбирается** этой дверью. Там
//! десятки гигабайт, скачанных пользователем: молчаливый перенос означал бы
//! двойной расход места и последующую потерю данных. Поведение при
//! недоступности — громкий отказ установки с путём и кодом ошибки, а не
//! подмена пути. Дверь не может сделать каталог записываемым; это умеет только
//! снятие метки целостности с тома или каталога.
//!
//! Порядок вызова обязателен: [`bootstrap`] — до создания Tauri-приложения
//! (его читает `load_config_early`), [`init`] — в `setup`, когда появился
//! `AppHandle` и можно подтвердить путь штатным способом.

use std::path::PathBuf;

use tauri::{AppHandle, Manager};

use ko_json_store::roots::{
    self, Candidate, LABEL_APP_DATA, LABEL_APP_SHARED, LABEL_SCRATCH,
};

use crate::infra::config::APP_DATA_DIR_NAME;

/// Имя подпапки рядом с exe, если системный `TEMP` закрыт.
const SCRATCH_DIR_NAME: &str = "tmp";

/// Имя подпапки для данных хоста внутри запасного корня. Разводит данные
/// приложения и данные плагинов, см. обоснование в заголовке модуля.
const APP_SUBDIR: &str = "app";

/// Папка рядом с исполняемым файлом. Единственная, о которой известно, что она
/// существует всегда: путь, по которому нас запустили, мы писать можем.
fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Базовый пользовательский каталог (`APPDATA`) без `AppHandle`.
fn user_base_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Штатный каталог данных приложения без `AppHandle`: `APPDATA/<identifier>`.
///
/// Совпадает с тем, что вернёт `app.path().app_data_dir()` — это подтверждается
/// в [`init`], но знать путь раньше `AppHandle` обязательно: конфиг читается
/// до создания приложения, иначе приложение писало бы в один каталог, а читало
/// конфиг из другого.
fn app_data_dir_env() -> PathBuf {
    user_base_dir().join(APP_DATA_DIR_NAME)
}

/// Лестницы кандидатов. `KING_ORCH_DATA_ROOT` идёт первым везде: это явное
/// указание пользователя, и оно не должно перебиваться догадками о том, что
/// «на этом диске больше свободного места».
fn app_data_candidates() -> Vec<Candidate> {
    let mut candidates = Vec::new();
    candidates.extend(roots::env_candidate("переменная окружения"));
    candidates.push(Candidate::new(app_data_dir_env(), "APPDATA"));
    candidates.push(Candidate::new(
        exe_dir().join("data").join(APP_SUBDIR),
        "папка приложения",
    ));
    candidates
}

/// Лестница временной папки. Системный `TEMP` берётся **как есть**, без
/// приклеивания подпапки: если он доступен, менять его не нужно, а любая
/// подпапка означала бы вложенность `tmp\tmp` при каждом повторном выборе.
/// Переопределение происходит только тогда, когда `TEMP` непригоден.
fn scratch_candidates() -> Vec<Candidate> {
    let mut candidates = vec![Candidate::new(
        std::env::temp_dir(),
        "системный TEMP",
    )];
    candidates.push(Candidate::new(
        exe_dir().join(SCRATCH_DIR_NAME),
        "папка приложения",
    ));
    if let Some(data) = roots::root(LABEL_APP_DATA) {
        candidates.push(Candidate::new(
            data.join(SCRATCH_DIR_NAME),
            "хранилище данных",
        ));
    }
    candidates
}

/// Лестница крупного хранилища `KingOrchData` намеренно **отсутствует**: там
/// десятки гигабайт, скачанных пользователем, и подменять путь по результату
/// пробы означало бы тихо удвоить занимаемое место. Недоступность этого каталога
/// проявляется громким отказом установки с путём и кодом ошибки — см. заголовок
/// модуля.

/// Разрешить корни до создания Tauri-приложения.
///
/// Вызывается первым действием `main()`: до этого момента читается конфиг и
/// запускаются плагины, и все они должны видеть уже принятое решение, а не
/// разрешать путь заново каждый сам.
///
/// [`LABEL_APP_SHARED`] здесь **не выбирается**: его лестница живёт в крейте,
/// и плагины вызывают `plugin_dir()` сами. Дублировать её здесь значило бы
/// завести второе место, где решается, — ровно то, что породило нынешний сбой.
pub fn bootstrap() {
    roots::choose(LABEL_APP_DATA, app_data_candidates());
    roots::choose(LABEL_SCRATCH, scratch_candidates());
    report_roots();
}

/// Подтвердить корни `AppHandle`, разнести данные и проиграть журналы.
///
/// Вызывается в `setup`. Штатный `app.path().app_data_dir()` добавляется в
/// лестницу как кандидат: если он недоступен, а ранее выбранный `APPDATA` —
/// доступен, ничего не меняется; если наоборот, выигрывает более надёжный путь.
pub fn init(app: &AppHandle) {
    let mut candidates = app_data_candidates();
    if let Ok(standard) = app.path().app_data_dir() {
        candidates.push(Candidate::new(standard, "app.path()"));
    }
    roots::invalidate(LABEL_APP_DATA);
    roots::choose(LABEL_APP_DATA, candidates);

    // Перенос данных ровно один раз на запуск: без него «починил» означал бы
    // «старые сессии исчезли». Источник не удаляется — вторая копия не мешает,
    // а откат остаётся возможным.
    roots::adopt(LABEL_APP_DATA, &app_data_dir_env(), "");

    // Журналы: значение, зафиксированное, но не дошедшее до файла, накладывается
    // на диск до того, как приложение начнёт читать данные.
    if let Some(root) = roots::root(LABEL_APP_DATA) {
        ko_json_store::journal::recover_tree(&root);
    }
    report_roots();
}

/// Короткая сводка по корням — одна строка на запуск, чтобы по логу было видно,
/// куда приложение пишет, без поиска по трём сообщениям.
fn report_roots() {
    for label in [LABEL_APP_DATA, LABEL_APP_SHARED, LABEL_SCRATCH] {
        if let Some(choice) = roots::choice(label) {
            log::info!(
                "[BOOT] корень {} = {} ({}), запись: {}",
                label,
                choice.path.display(),
                choice.source,
                if choice.writable { "да" } else { "НЕТ" }
            );
        }
    }
}

/// Каталог данных приложения. Обязан вызываться после [`bootstrap`].
pub fn app_data_root() -> PathBuf {
    roots::root(LABEL_APP_DATA).unwrap_or_else(|| app_data_dir_env())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_data_dir_env_matches_identifier() {
        assert!(app_data_dir_env().ends_with(APP_DATA_DIR_NAME));
    }

    #[test]
    fn scratch_ladder_starts_at_system_temp_without_nesting() {
        let ladder = scratch_candidates();
        assert_eq!(
            ladder[0].path,
            std::env::temp_dir(),
            "доступный TEMP обязан браться как есть — иначе получим tmp\\tmp"
        );
        assert!(ladder
            .iter()
            .any(|c| c.path.ends_with(SCRATCH_DIR_NAME)));
    }

    #[test]
    fn app_data_fallback_is_separate_from_plugin_dirs() {
        let ladder = app_data_candidates();
        let fallback = ladder.last().unwrap();
        // Запасной корень хоста и запасной корень плагинов — один каталог
        // `data`, но данные хоста обязаны лежать в `data/app`. Иначе перенос
        // плагинов «заполнял» каталог раньше хоста, и данные хоста терялись.
        assert_eq!(
            fallback.path.file_name().unwrap().to_str(),
            Some(APP_SUBDIR),
            "данные приложения обязаны быть в подпапке, а не вперемешку с плагинами"
        );
        assert_eq!(
            fallback.path.parent().unwrap().file_name().unwrap().to_str(),
            Some("data"),
            "общий с плагинами каталог остаётся родителем — но данные хоста лежат глубже"
        );
        assert_eq!(fallback.source, "папка приложения");
    }
}