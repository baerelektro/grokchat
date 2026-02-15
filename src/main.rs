// Подключаем внутренние модули проекта.
mod app;
mod config;
mod handlers;
mod logging;
mod p2p;

// Стандартный тип ошибки для удобного проброса ошибок вверх.
use std::error::Error;

// Главная точка входа Tokio runtime.
// Здесь мы просто передаём управление в app::run(),
// где лежит вся бизнес-логика приложения.
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    app::run().await
}
