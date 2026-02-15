// Коротко про этот файл:
// 1) Подключает все внутренние модули проекта.
// 2) Запускает Tokio runtime.
// 3) Передаёт управление в app::run().
// Подключаем внутренние модули проекта.
mod app;
mod api_types;
mod config;
mod gateway;
mod handlers;
mod logging;
mod p2p;
mod sync;

// Стандартный тип ошибки для удобного проброса ошибок вверх.
use std::error::Error;

// Главная точка входа Tokio runtime.
// Здесь мы просто передаём управление в app::run(),
// где лежит вся бизнес-логика приложения.
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    app::run().await
}
