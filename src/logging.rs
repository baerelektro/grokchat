// Коротко про этот файл:
// 1) Запускает фоновый writer логов в файл.
// 2) Принимает строки логов через канал.
// 3) Добавляет timestamp к каждому событию.
// Простейшая подсистема логирования: принимаем строки через канал и пишем в файл.
use std::{path::PathBuf, time::SystemTime};
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

// Фоновая задача, которая постоянно читает сообщения из канала и дописывает их в лог.
pub async fn run_log_writer(log_path: PathBuf, mut receiver: UnboundedReceiver<String>) {
    // Открываем файл в режиме append: старые логи не теряем,
    // а дописываем новые строки в конец.
    match OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .await
    {
        Ok(mut file) => {
            // Пока канал жив, получаем новые строки.
            while let Some(mut line) = receiver.recv().await {
                // Гарантируем перевод строки в конце, чтобы лог был читаемым.
                if !line.ends_with('\n') {
                    line.push('\n');
                }
                // Пишем строку в файл.
                // Если запись не удалась, выводим ошибку в stderr,
                // но не роняем весь процесс (логгер не должен убивать приложение).
                if let Err(err) = file.write_all(line.as_bytes()).await {
                    eprintln!("Не удалось записать в лог {}: {err}", log_path.display());
                }
            }
        }
        Err(err) => {
            eprintln!("Не удалось открыть лог {}: {err}", log_path.display());
        }
    }
}

// Удобный helper: добавляет timestamp и отправляет событие в канал логгера.
pub fn log_event(sender: &UnboundedSender<String>, message: String) {
    // Время в формате UNIX epoch (секунды + миллисекунды).
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let timestamp = format!("{}.{:03}", now.as_secs(), now.subsec_millis());
    // Игнорируем ошибку send (например, если канал уже закрыт при завершении процесса).
    // Это штатно при остановке приложения.
    let _ = sender.send(format!("[{timestamp}] {message}"));
}
