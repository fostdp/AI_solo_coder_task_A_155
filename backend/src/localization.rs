use crate::models::{SensorReading, SourceLocalizationResult, UrnDevice};
use chrono::Utc;
use std::collections::HashMap;
use std::f64::consts::PI;

pub struct Beamformer {
    sound_speed: f64,
    resolution: f64,
    max_distance: f64,
    confidence_threshold: f64,
}

impl Beamformer {
    pub fn new(sound_speed: f64, resolution: f64, max_distance: f64, confidence_threshold: f64) -> Self {
        Self {
            sound_speed,
            resolution,
            max_distance,
            confidence_threshold,
        }
    }

    pub fn locate_source(
        &self,
        readings: &[(SensorReading, UrnDevice)],
        source_id: u64,
    ) -> Option<SourceLocalizationResult> {
        if readings.len() < 3 {
            return None;
        }

        let tdoa_matrix = self.compute_tdoa_matrix(readings);
        let (best_x, best_y, best_z, max_power) = self.delay_and_sum(readings, &tdoa_matrix);

        let center_x = readings.iter().map(|(_, d)| d.deployment_x).sum::<f64>() / readings.len() as f64;
        let center_y = readings.iter().map(|(_, d)| d.deployment_y).sum::<f64>() / readings.len() as f64;

        let dx = best_x - center_x;
        let dy = best_y - center_y;
        let dz = best_z;

        let distance = (dx * dx + dy * dy + dz * dz).sqrt();
        let bearing = dy.atan2(dx).to_degrees();
        let bearing_normalized = if bearing < 0.0 { bearing + 360.0 } else { bearing };
        let elevation = dz.atan2((dx * dx + dy * dy).sqrt()).to_degrees();

        let normalized_power = (max_power / readings.len() as f64).min(1.0).max(0.0);
        let confidence = self.calculate_confidence(readings.len(), normalized_power, distance);

        let used_devices = readings.iter().map(|(r, _)| r.device_id).collect();

        Some(SourceLocalizationResult {
            timestamp: Utc::now(),
            source_id,
            source_x: best_x,
            source_y: best_y,
            source_z: best_z,
            bearing_angle: bearing_normalized,
            elevation_angle: elevation,
            distance_estimate: distance,
            confidence,
            tdoa_matrix: tdoa_matrix.clone(),
            beamformed_power: max_power,
            used_devices,
        })
    }

    fn compute_tdoa_matrix(&self, readings: &[(SensorReading, UrnDevice)]) -> Vec<Vec<f64>> {
        let n = readings.len();
        let mut matrix = vec![vec![0.0; n]; n];

        for i in 0..n {
            for j in 0..n {
                if i != j {
                    let dist_i = self.estimate_distance_from_spl(&readings[i].0);
                    let dist_j = self.estimate_distance_from_spl(&readings[j].0);
                    let tdoa = (dist_i - dist_j) / self.sound_speed;
                    matrix[i][j] = tdoa;
                }
            }
        }
        matrix
    }

    fn estimate_distance_from_spl(&self, reading: &SensorReading) -> f64 {
        let reference_spl = 94.0;
        let reference_distance = 1.0;
        let attenuation = 6.0;
        let spl_diff = reference_spl - reading.sound_pressure_level;
        reference_distance * 10.0_f64.powf(spl_diff / attenuation)
    }

    fn delay_and_sum(
        &self,
        readings: &[(SensorReading, UrnDevice)],
        _tdoa_matrix: &[Vec<f64>],
    ) -> (f64, f64, f64, f64) {
        let num_points = (self.max_distance / self.resolution) as usize;
        let mut max_power = 0.0;
        let mut best_x = 0.0;
        let mut best_y = 0.0;
        let mut best_z = 0.0;

        let center_x = readings.iter().map(|(_, d)| d.deployment_x).sum::<f64>() / readings.len() as f64;
        let center_y = readings.iter().map(|(_, d)| d.deployment_y).sum::<f64>() / readings.len() as f64;

        for i in 0..num_points {
            let angle = 2.0 * PI * i as f64 / num_points as f64;
            for r in (0..num_points).step_by(5) {
                let radius = r as f64 * self.resolution * 5.0;
                if radius < 1.0 || radius > self.max_distance {
                    continue;
                }

                let test_x = center_x + radius * angle.cos();
                let test_y = center_y + radius * angle.sin();
                let test_z = -radius * 0.3;

                let power = self.compute_beam_power(readings, test_x, test_y, test_z);
                if power > max_power {
                    max_power = power;
                    best_x = test_x;
                    best_y = test_y;
                    best_z = test_z;
                }
            }
        }

        (best_x, best_y, best_z, max_power)
    }

    fn compute_beam_power(
        &self,
        readings: &[(SensorReading, UrnDevice)],
        x: f64,
        y: f64,
        z: f64,
    ) -> f64 {
        let mut total_power = 0.0;
        let reference_device = &readings[0].1;
        let reference_dist = Self::distance_3d(
            reference_device.deployment_x,
            reference_device.deployment_y,
            reference_device.deployment_z,
            x, y, z,
        );

        for (reading, device) in readings {
            let dist = Self::distance_3d(
                device.deployment_x,
                device.deployment_y,
                device.deployment_z,
                x, y, z,
            );
            let time_diff = (dist - reference_dist) / self.sound_speed;
            let phase = 2.0 * PI * reading.resonance_frequency * time_diff;
            let amplitude = reading.sound_pressure_level * 10.0_f64.powf(-dist / 100.0);
            total_power += amplitude * phase.cos();
        }

        total_power.abs()
    }

    fn distance_3d(x1: f64, y1: f64, z1: f64, x2: f64, y2: f64, z2: f64) -> f64 {
        let dx = x2 - x1;
        let dy = y2 - y1;
        let dz = z2 - z1;
        (dx * dx + dy * dy + dz * dz).sqrt()
    }

    fn calculate_confidence(&self, num_sensors: usize, power_ratio: f64, distance: f64) -> f64 {
        let sensor_factor = (num_sensors as f64 / 6.0).min(1.0);
        let distance_factor = (1.0 - distance / self.max_distance).max(0.2);
        let power_factor = power_ratio;
        sensor_factor * distance_factor * power_factor
    }
}

pub struct TDOALocalizer {
    sound_speed: f64,
}

impl TDOALocalizer {
    pub fn new(sound_speed: f64) -> Self {
        Self { sound_speed }
    }

    pub fn multilaterate_2d(
        &self,
        devices: &[UrnDevice],
        tdoa_values: &HashMap<(usize, usize), f64>,
    ) -> Option<(f64, f64, f64)> {
        if devices.len() < 3 {
            return None;
        }

        let reference_idx = 0;
        let ref_device = &devices[reference_idx];

        let mut best_pos = (0.0, 0.0, 0.0);
        let mut min_error = f64::MAX;

        let search_range = 1000.0;
        let grid_steps = 100;
        let step = search_range * 2.0 / grid_steps as f64;

        for i in 0..grid_steps {
            let x = ref_device.deployment_x - search_range + i as f64 * step;
            for j in 0..grid_steps {
                let y = ref_device.deployment_y - search_range + j as f64 * step;

                let error = self.calculate_tdoa_error(devices, tdoa_values, reference_idx, x, y, 0.0);
                if error < min_error {
                    min_error = error;
                    best_pos = (x, y, 0.0);
                }
            }
        }

        let confidence = (1.0 - min_error / (devices.len() as f64 * 0.01)).max(0.1).min(0.99);
        Some((best_pos.0, best_pos.1, confidence))
    }

    fn calculate_tdoa_error(
        &self,
        devices: &[UrnDevice],
        tdoa_values: &HashMap<(usize, usize), f64>,
        _ref_idx: usize,
        x: f64,
        y: f64,
        z: f64,
    ) -> f64 {
        let mut total_error = 0.0;
        let mut count = 0;

        for ((i, j), measured_tdoa) in tdoa_values {
            let dist_i = Self::dist(&devices[*i], x, y, z);
            let dist_j = Self::dist(&devices[*j], x, y, z);
            let predicted_tdoa = (dist_i - dist_j) / self.sound_speed;
            let error = (predicted_tdoa - measured_tdoa).powi(2);
            total_error += error;
            count += 1;
        }

        if count == 0 { 0.0 } else { total_error / count as f64 }
    }

    fn dist(device: &UrnDevice, x: f64, y: f64, z: f64) -> f64 {
        let dx = device.deployment_x - x;
        let dy = device.deployment_y - y;
        let dz = device.deployment_z - z;
        (dx * dx + dy * dy + dz * dz).sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn create_test_devices() -> Vec<UrnDevice> {
        vec![
            UrnDevice { device_id: 1, device_name: "U1".to_string(), deployment_x: 0.0, deployment_y: 0.0, deployment_z: 0.0, urn_volume: 0.05, neck_radius: 0.05, neck_length: 0.1 },
            UrnDevice { device_id: 2, device_name: "U2".to_string(), deployment_x: 10.0, deployment_y: 0.0, deployment_z: 0.0, urn_volume: 0.05, neck_radius: 0.05, neck_length: 0.1 },
            UrnDevice { device_id: 3, device_name: "U3".to_string(), deployment_x: 5.0, deployment_y: 10.0, deployment_z: 0.0, urn_volume: 0.05, neck_radius: 0.05, neck_length: 0.1 },
        ]
    }

    #[test]
    fn test_beamformer_creation() {
        let bf = Beamformer::new(1500.0, 1.0, 500.0, 0.5);
        assert_eq!(bf.sound_speed, 1500.0);
    }
}
