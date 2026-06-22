mod acoustics;
mod alerts;
mod config;
mod handlers;
mod localization;
mod models;
mod mqtt_subscriber;
mod store;
mod websocket;

use acoustics::AcousticAnalyzer;
use alerts::AlertManager;
use axum::routing::{get, post};
use axum::Router;
use dashmap::DashMap;
use handlers::AppState;
use localization::Beamformer;
use models::UrnDevice;
use mqtt_subscriber::MqttSubscriber;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::signal;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

fn load_config() -> config::Config {
    config::Config {
        server: config::ServerConfig {
            host: "0.0.0.0".to_string(),
            port: 8080,
            static_dir: std::path::PathBuf::from("../frontend"),
        },
        mqtt: config::MqttConfig {
            broker: "localhost".to_string(),
            port: 1883,
            client_id: "urn_acoustics_backend".to_string(),
            topic: "urn/sensors/#".to_string(),
            username: None,
            password: None,
        },
        clickhouse: config::ClickHouseConfig {
            url: "http://localhost:8123".to_string(),
            database: "urn_acoustics".to_string(),
            user: "default".to_string(),
            password: "".to_string(),
        },
        acoustics: config::AcousticsConfig {
            speed_of_sound: 343.0,
            default_urn_volume: 0.05,
            default_neck_radius: 0.05,
            default_neck_length: 0.1,
            drift_warning_threshold_percent: 5.0,
            drift_critical_threshold_percent: 15.0,
        },
        localization: config::LocalizationConfig {
            sound_speed_soil: 1500.0,
            beamforming_resolution: 1.0,
            max_localization_distance: 500.0,
            localization_confidence_threshold: 0.3,
        },
        alert: config::AlertConfig {
            frequency_drift_warning: 5.0,
            localization_bias_warning: 50.0,
            cooldown_seconds: 30,
        },
    }
}

mod config {
    use serde::Deserialize;
    use std::path::PathBuf;

    #[derive(Debug, Deserialize, Clone)]
    pub struct Config {
        pub server: ServerConfig,
        pub mqtt: MqttConfig,
        pub clickhouse: ClickHouseConfig,
        pub acoustics: AcousticsConfig,
        pub localization: LocalizationConfig,
        pub alert: AlertConfig,
    }

    #[derive(Debug, Deserialize, Clone)]
    pub struct ServerConfig {
        pub host: String,
        pub port: u16,
        pub static_dir: PathBuf,
    }

    #[derive(Debug, Deserialize, Clone)]
    pub struct MqttConfig {
        pub broker: String,
        pub port: u16,
        pub client_id: String,
        pub topic: String,
        pub username: Option<String>,
        pub password: Option<String>,
    }

    #[derive(Debug, Deserialize, Clone)]
    pub struct ClickHouseConfig {
        pub url: String,
        pub database: String,
        pub user: String,
        pub password: String,
    }

    #[derive(Debug, Deserialize, Clone)]
    pub struct AcousticsConfig {
        pub speed_of_sound: f64,
        pub default_urn_volume: f64,
        pub default_neck_radius: f64,
        pub default_neck_length: f64,
        pub drift_warning_threshold_percent: f64,
        pub drift_critical_threshold_percent: f64,
    }

    #[derive(Debug, Deserialize, Clone)]
    pub struct LocalizationConfig {
        pub sound_speed_soil: f64,
        pub beamforming_resolution: f64,
        pub max_localization_distance: f64,
        pub localization_confidence_threshold: f64,
    }

    #[derive(Debug, Deserialize, Clone)]
    pub struct AlertConfig {
        pub frequency_drift_warning: f64,
        pub localization_bias_warning: f64,
        pub cooldown_seconds: u64,
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "urn_acoustics_backend=info,tower_http=info,axum=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = load_config();

    let (tx, _rx) = broadcast::channel::<models::WebSocketMessage>(256);

    let alert_manager = AlertManager::new(
        config.alert.frequency_drift_warning,
        config.alert.localization_bias_warning,
        config.alert.cooldown_seconds,
        tx.clone(),
    );

    let store = store::ClickHouseStore::new(
        &config.clickhouse.url,
        &config.clickhouse.database,
        &config.clickhouse.user,
        &config.clickhouse.password,
    );

    let devices = Arc::new(DashMap::<u32, UrnDevice>::new());

    let default_devices = vec![
        UrnDevice {
            device_id: 1,
            device_name: "瓮听-东北角".to_string(),
            deployment_x: -50.0,
            deployment_y: -50.0,
            deployment_z: -2.0,
            urn_volume: 0.05,
            neck_radius: 0.05,
            neck_length: 0.1,
        },
        UrnDevice {
            device_id: 2,
            device_name: "瓮听-东南角".to_string(),
            deployment_x: 50.0,
            deployment_y: -50.0,
            deployment_z: -2.0,
            urn_volume: 0.05,
            neck_radius: 0.05,
            neck_length: 0.1,
        },
        UrnDevice {
            device_id: 3,
            device_name: "瓮听-西南角".to_string(),
            deployment_x: 50.0,
            deployment_y: 50.0,
            deployment_z: -2.0,
            urn_volume: 0.05,
            neck_radius: 0.05,
            neck_length: 0.1,
        },
        UrnDevice {
            device_id: 4,
            device_name: "瓮听-西北角".to_string(),
            deployment_x: -50.0,
            deployment_y: 50.0,
            deployment_z: -2.0,
            urn_volume: 0.05,
            neck_radius: 0.05,
            neck_length: 0.1,
        },
        UrnDevice {
            device_id: 5,
            device_name: "瓮听-正中央".to_string(),
            deployment_x: 0.0,
            deployment_y: 0.0,
            deployment_z: -2.0,
            urn_volume: 0.08,
            neck_radius: 0.06,
            neck_length: 0.12,
        },
    ];

    for device in default_devices {
        info!("注册默认设备: {} (ID={})", device.device_name, device.device_id);
        devices.insert(device.device_id, device);
    }

    let acoustic_config = config.acoustics.clone();
    let analyzer = AcousticAnalyzer::new(
        acoustic_config.speed_of_sound,
        acoustic_config.drift_warning_threshold_percent,
        acoustic_config.drift_critical_threshold_percent,
    );

    let loc_config = config.localization.clone();
    let beamformer = Beamformer::new(
        loc_config.sound_speed_soil,
        loc_config.beamforming_resolution,
        loc_config.max_localization_distance,
        loc_config.localization_confidence_threshold,
    );

    let mqtt_subscriber = Arc::new(MqttSubscriber::new(
        store.clone(),
        analyzer,
        beamformer,
        alert_manager.clone(),
        devices.clone(),
    ));

    let mqtt_config = config.mqtt.clone();
    tokio::spawn(async move {
        info!("启动MQTT订阅服务...");
        mqtt_subscriber.run(&mqtt_config).await;
    });

    let app_state = AppState {
        store: store.clone(),
        alert_manager: alert_manager.clone(),
        devices: devices.clone(),
        config: config.clone(),
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let static_dir_path = config.server.static_dir.clone();
    let serve_dir = ServeDir::new(static_dir_path);

    let app = Router::new()
        .route("/api/health", get(handlers::health_check))
        .route("/api/devices", get(handlers::get_devices).post(handlers::create_device))
        .route("/api/devices/:id", get(handlers::get_device))
        .route("/api/sensor-data", get(handlers::get_sensor_data))
        .route("/api/localizations", get(handlers::get_recent_localizations))
        .route("/api/alerts", get(handlers::get_recent_alerts))
        .route("/api/medium-properties", get(handlers::get_medium_properties))
        .route("/api/resonance/calculate", get(handlers::calculate_resonance))
        .route("/api/simulate/reading", post(handlers::simulate_reading))
        .route("/api/ws/broadcast-test", get(handlers::broadcast_test_message))
        .route("/ws", get(websocket::websocket_handler))
        .fallback_service(serve_dir)
        .layer(cors)
        .with_state(app_state);

    let addr = format!("{}:{}", config.server.host, config.server.port);
    info!("瓮听声学系统后端服务启动中: {}", addr);

    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            error!("无法绑定地址 {}: {}", addr, e);
            std::process::exit(1);
        }
    };

    match axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
    {
        Ok(_) => info!("服务器正常关闭"),
        Err(e) => error!("服务器错误: {}", e),
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("收到关闭信号，正在优雅停止服务...");
}
