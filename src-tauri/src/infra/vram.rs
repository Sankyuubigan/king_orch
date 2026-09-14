//! 🔔 Неблокирующее уведомление об итоге pre-flight VRAM.
//!
//! Раньше при нехватке видеопамяти показывался БЛОКИРУЮЩИЙ диалог «Отмена /
//! Запустить с выгрузкой в ОЗУ» (mpsc-ожидание в агентском цикле). Это
//! останавливало запуск модели на неопределённое время и порождало ложные
//! срабатывания на неточной оценке. Теперь pre-flight сам урезает -ngl по
//! оценке (`infra::vram_estimate`, SSOT) и отправляет юзеру окно-УВЕДОМЛЕНИЕ
//! с фактами и одной кнопкой «ОК» — запуск не блокируется.

use crate::infra::event_bus::{AgentEvent, global_bus};

/// Отправить юзеру уведомление о VRAM (non-blocking, одна кнопка «ОК»).
pub fn notify_vram(note: String) {
    global_bus().publish(AgentEvent::VramNotice { note });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notify_publishes_vram_notice_to_bus() {
        let received = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let r = received.clone();
        global_bus().subscribe(std::sync::Arc::new(move |e| {
            if let AgentEvent::VramNotice { note } = e {
                r.lock().unwrap().push(note.clone());
            }
        }));
        notify_vram("тест".to_string());
        let got = received.lock().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0], "тест");
    }
}