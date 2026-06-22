use crate::models::{Alert, ResonanceAnalysisResult, SensorReading, SourceLocalizationResult, UrnDevice, MediumProperty};
use clickhouse::Client;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct ClickHouseStore {
    client: Client,
    database: String,
}

impl ClickHouseStore {
    pub fn new(url: &str, database: &str, user: &str, password: &str) -> Self {
        let client = Client::default()
            .with_url(url)
            .with_database(database)
            .with_user(user)
            .with_password(password);

        Self {
            client,
            database: database.to_string(),
        }
    }

    pub async fn insert_sensor_reading(&self, reading: &SensorReading) -> Result<(), clickhouse::error::Error> {
        let mut inserter = self.client
            .insert("sensor_data")?;

        inserter
            .write(reading.timestamp)
            .write(reading.device_id)
            .write(reading.sound_pressure_level)
            .write(reading.resonance_frequency)
            .write(reading.source_direction)
            .write(reading.medium_density)
            .write(reading.temperature)
            .write(reading.humidity)
            .commit()
            .await?;

        Ok(())
    }

    pub async fn insert_resonance_analysis(
        &self,
        analysis: &ResonanceAnalysisResult,
    ) -> Result<(), clickhouse::error::Error> {
        let mut inserter = self.client
            .insert("resonance_analysis")?;

        inserter
            .write(analysis.timestamp)
            .write(analysis.device_id)
            .write(analysis.measured_resonance_freq)
            .write(analysis.theoretical_resonance_freq)
            .write(analysis.gain_db)
            .write(analysis.quality_factor)
            .write(analysis.frequency_drift)
            .write(analysis.drift_percent)
            .write(analysis.is_anomaly)
            .commit()
            .await?;

        Ok(())
    }

    pub async fn insert_localization(
        &self,
        loc: &SourceLocalizationResult,
    ) -> Result<(), clickhouse::error::Error> {
        let tdoa_str = serde_json::to_string(&loc.tdoa_matrix).unwrap_or_default();

        let mut inserter = self.client
            .insert("source_localization")?;

        inserter
            .write(loc.timestamp)
            .write(loc.source_id)
            .write(loc.source_x)
            .write(loc.source_y)
            .write(loc.source_z)
            .write(loc.bearing_angle)
            .write(loc.elevation_angle)
            .write(loc.distance_estimate)
            .write(loc.confidence)
            .write(tdoa_str)
            .write(loc.beamformed_power)
            .write(loc.used_devices.clone())
            .commit()
            .await?;

        Ok(())
    }

    pub async fn insert_alert(&self, alert: &Alert) -> Result<(), clickhouse::error::Error> {
        let mut inserter = self.client
            .insert("alerts")?;

        inserter
            .write(alert.timestamp)
            .write(alert.alert_id)
            .write(alert.alert_type.clone())
            .write(alert.severity.clone())
            .write(alert.device_id)
            .write(alert.message.clone())
            .write(alert.details.clone())
            .write(alert.is_resolved)
            .commit()
            .await?;

        Ok(())
    }

    pub async fn get_devices(&self) -> Result<Vec<UrnDevice>, clickhouse::error::Error> {
        let query = "SELECT device_id, device_name, deployment_x, deployment_y, deployment_z,
                     urn_volume, neck_radius, neck_length FROM urn_devices";

        let devices = self.client
            .query(query)
            .fetch_all::<UrnDevice>()
            .await?;

        Ok(devices)
    }

    pub async fn get_recent_sensor_data(
        &self,
        device_id: Option<u32>,
        limit: u32,
    ) -> Result<Vec<SensorReading>, clickhouse::error::Error> {
        let query = if let Some(id) = device_id {
            format!(
                "SELECT timestamp, device_id, sound_pressure_level, resonance_frequency,
                 source_direction, medium_density, temperature, humidity
                 FROM sensor_data WHERE device_id = {}
                 ORDER BY timestamp DESC LIMIT {}",
                id, limit
            )
        } else {
            format!(
                "SELECT timestamp, device_id, sound_pressure_level, resonance_frequency,
                 source_direction, medium_density, temperature, humidity
                 FROM sensor_data ORDER BY timestamp DESC LIMIT {}",
                limit
            )
        };

        let readings = self.client
            .query(&query)
            .fetch_all::<SensorReading>()
            .await?;

        Ok(readings)
    }

    pub async fn get_medium_properties(&self) -> Result<Vec<MediumProperty>, clickhouse::error::Error> {
        let query = "SELECT medium_type, density, sound_speed, attenuation_coeff FROM medium_properties";
        let props = self.client
            .query(query)
            .fetch_all::<MediumProperty>()
            .await?;
        Ok(props)
    }

    pub async fn get_recent_alerts(&self, limit: u32) -> Result<Vec<Alert>, clickhouse::error::Error> {
        let query = format!(
            "SELECT timestamp, alert_id, alert_type, severity, device_id, message, details, is_resolved
             FROM alerts ORDER BY timestamp DESC LIMIT {}",
            limit
        );

        let alerts = self.client
            .query(&query)
            .fetch_all::<Alert>()
            .await?;

        Ok(alerts)
    }

    pub async fn get_recent_localizations(&self, limit: u32) -> Result<Vec<SourceLocalizationResult>, clickhouse::error::Error> {
        let query = format!(
            "SELECT timestamp, source_id, source_x, source_y, source_z, bearing_angle,
             elevation_angle, distance_estimate, confidence, tdoa_matrix, beamformed_power,
             used_devices FROM source_localization ORDER BY timestamp DESC LIMIT {}",
            limit
        );

        let results = self.client
            .query(&query)
            .fetch_all::<SourceLocalizationResult>()
            .await?;

        Ok(results)
    }
}

pub type SharedStore = Arc<Mutex<ClickHouseStore>>;
