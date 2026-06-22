use crate::acoustics::{HelmholtzResonator, AcousticAnalyzer};
use crate::alerts::AlertManager;
use crate::config::Config;
use crate::models::{SensorReading, UrnDevice, WebSocketMessage, MediumProperty, ResonanceAnalysisResult};
use crate::store::ClickHouseStore;
use axum::extract::{Query, State, Path};
use axum::http::StatusCode;
use axum::response::Json;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::info;

#[derive(Clone)]
pub struct AppState {
    pub store: ClickHouseStore,
    pub alert_manager: Arc<AlertManager>,
    pub devices: Arc<DashMap<u32, UrnDevice>>,
    pub config: Config,
}

#[derive(Debug, Deserialize)]
pub struct PaginationQuery {
    pub limit: Option<u32>,
    pub device_id: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}

impl<T> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn err(msg: &str) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(msg.to_string()),
        }
    }
}

pub async fn health_check() -> Json<ApiResponse<String>> {
    Json(ApiResponse::ok("ok".to_string()))
}

pub async fn get_devices(
    State(state): State<AppState>,
) -> Json<ApiResponse<Vec<UrnDevice>>> {
    let devices: Vec<UrnDevice> = state.devices.iter().map(|d| d.value().clone()).collect();
    Json(ApiResponse::ok(devices))
}

pub async fn get_sensor_data(
    State(state): State<AppState>,
    Query(params): Query<PaginationQuery>,
) -> Json<ApiResponse<Vec<SensorReading>>> {
    let limit = params.limit.unwrap_or(100);
    match state.store.get_recent_sensor_data(params.device_id, limit).await {
        Ok(data) => Json(ApiResponse::ok(data)),
        Err(e) => Json(ApiResponse::err(&format!("查询传感器数据失败: {}", e))),
    }
}

pub async fn get_recent_localizations(
    State(state): State<AppState>,
    Query(params): Query<PaginationQuery>,
) -> Json<ApiResponse<Vec<crate::models::SourceLocalizationResult>>> {
    let limit = params.limit.unwrap_or(50);
    match state.store.get_recent_localizations(limit).await {
        Ok(data) => Json(ApiResponse::ok(data)),
        Err(e) => Json(ApiResponse::err(&format!("查询定位结果失败: {}", e))),
    }
}

pub async fn get_recent_alerts(
    State(state): State<AppState>,
    Query(params): Query<PaginationQuery>,
) -> Json<ApiResponse<Vec<crate::models::Alert>>> {
    let limit = params.limit.unwrap_or(50);
    match state.store.get_recent_alerts(limit).await {
        Ok(data) => Json(ApiResponse::ok(data)),
        Err(e) => Json(ApiResponse::err(&format!("查询告警失败: {}", e))),
    }
}

pub async fn get_medium_properties(
    State(state): State<AppState>,
) -> Json<ApiResponse<Vec<MediumProperty>>> {
    match state.store.get_medium_properties().await {
        Ok(data) => Json(ApiResponse::ok(data)),
        Err(e) => Json(ApiResponse::err(&format!("查询介质参数失败: {}", e))),
    }
}

#[derive(Debug, Deserialize)]
pub struct ResonanceCalcQuery {
    pub frequency: f64,
    pub volume: Option<f64>,
    pub neck_radius: Option<f64>,
    pub neck_length: Option<f64>,
    pub speed_of_sound: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct ResonanceCalcResult {
    pub resonance_frequency: f64,
    pub gain_db: f64,
    pub quality_factor: f64,
    pub gain_curve: Vec<(f64, f64)>,
}

pub async fn calculate_resonance(
    State(state): State<AppState>,
    Query(params): Query<ResonanceCalcQuery>,
) -> Json<ApiResponse<ResonanceCalcResult>> {
    let config = &state.config.acoustics;
    let resonator = HelmholtzResonator::new(
        params.volume.unwrap_or(config.default_urn_volume),
        params.neck_radius.unwrap_or(config.default_neck_radius),
        params.neck_length.unwrap_or(config.default_neck_length),
        params.speed_of_sound.unwrap_or(config.speed_of_sound),
    );

    let resonance_freq = resonator.resonance_frequency();
    let gain_db = resonator.gain_db(params.frequency);
    let q_factor = resonator.quality_factor();

    let mut gain_curve = Vec::new();
    for i in 0..100 {
        let f = 20.0 + i as f64 * 10.0;
        let g = resonator.gain_db(f);
        gain_curve.push((f, g));
    }

    Json(ApiResponse::ok(ResonanceCalcResult {
        resonance_frequency: resonance_freq,
        gain_db,
        quality_factor: q_factor,
        gain_curve,
    }))
}

#[derive(Debug, Deserialize)]
pub struct SimulateReadingBody {
    pub device_id: u32,
    pub sound_pressure_level: f64,
    pub resonance_frequency: f64,
    pub source_direction: f64,
    pub medium_density: f64,
    pub temperature: f64,
    pub humidity: f64,
}

pub async fn simulate_reading(
    State(state): State<AppState>,
    Json(body): Json<SimulateReadingBody>,
) -> (StatusCode, Json<ApiResponse<ResonanceAnalysisResult>>) {
    let reading = SensorReading {
        timestamp: chrono::Utc::now(),
        device_id: body.device_id,
        sound_pressure_level: body.sound_pressure_level,
        resonance_frequency: body.resonance_frequency,
        source_direction: body.source_direction,
        medium_density: body.medium_density,
        temperature: body.temperature,
        humidity: body.humidity,
    };

    let device = match state.devices.get(&body.device_id) {
        Some(d) => d.value().clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ApiResponse::err(&format!("设备 {} 不存在", body.device_id))),
            );
        }
    };

    let acoustic_config = &state.config.acoustics;
    let analyzer = AcousticAnalyzer::new(
        acoustic_config.speed_of_sound,
        acoustic_config.drift_warning_threshold_percent,
        acoustic_config.drift_critical_threshold_percent,
    );

    let analysis = analyzer.analyze(&reading, &device);

    if let Err(e) = state.store.insert_sensor_reading(&reading).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::err(&format!("存储传感器数据失败: {}", e))),
        );
    }

    if let Err(e) = state.store.insert_resonance_analysis(&analysis).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::err(&format!("存储分析结果失败: {}", e))),
        );
    }

    state.alert_manager.broadcast_sensor_data(&reading);
    state.alert_manager.broadcast_resonance(&analysis);

    if let Some(alert) = state.alert_manager.check_resonance(&analysis) {
        let _ = state.store.insert_alert(&alert).await;
    }

    info!("已处理模拟传感器读数: 设备={}, SPL={:.1}dB", body.device_id, body.sound_pressure_level);

    (StatusCode::OK, Json(ApiResponse::ok(analysis)))
}

#[derive(Debug, Deserialize)]
pub struct CreateDeviceBody {
    pub device_id: u32,
    pub device_name: String,
    pub deployment_x: f64,
    pub deployment_y: f64,
    pub deployment_z: f64,
    pub urn_volume: f64,
    pub neck_radius: f64,
    pub neck_length: f64,
}

pub async fn create_device(
    State(state): State<AppState>,
    Json(body): Json<CreateDeviceBody>,
) -> (StatusCode, Json<ApiResponse<UrnDevice>>) {
    let device = UrnDevice {
        device_id: body.device_id,
        device_name: body.device_name.clone(),
        deployment_x: body.deployment_x,
        deployment_y: body.deployment_y,
        deployment_z: body.deployment_z,
        urn_volume: body.urn_volume,
        neck_radius: body.neck_radius,
        neck_length: body.neck_length,
    };

    state.devices.insert(body.device_id, device.clone());

    info!("已注册瓮听设备: {} ({})", body.device_name, body.device_id);
    (StatusCode::CREATED, Json(ApiResponse::ok(device)))
}

pub async fn get_device(
    State(state): State<AppState>,
    Path(device_id): Path<u32>,
) -> (StatusCode, Json<ApiResponse<UrnDevice>>) {
    match state.devices.get(&device_id) {
        Some(d) => (StatusCode::OK, Json(ApiResponse::ok(d.value().clone()))),
        None => (StatusCode::NOT_FOUND, Json(ApiResponse::err("设备不存在"))),
    }
}

pub async fn broadcast_test_message(
    State(state): State<AppState>,
) -> Json<ApiResponse<String>> {
    let msg = WebSocketMessage::new(
        "test",
        serde_json::json!({ "message": "这是一条测试广播消息", "time": chrono::Utc::now().to_rfc3339() }),
    );
    let tx = state.alert_manager.sender();
    match tx.send(msg) {
        Ok(n) => Json(ApiResponse::ok(format!("已广播给 {} 个订阅者", n))),
        Err(e) => Json(ApiResponse::err(&format!("广播失败: {}", e))),
    }
}
