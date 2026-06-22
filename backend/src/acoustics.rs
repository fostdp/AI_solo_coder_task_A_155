use crate::models::{ResonanceAnalysisResult, SensorReading, UrnDevice};
use chrono::Utc;
use std::f64::consts::PI;

pub struct HelmholtzResonator {
    pub volume: f64,
    pub neck_radius: f64,
    pub neck_length: f64,
    pub speed_of_sound: f64,
}

impl HelmholtzResonator {
    pub fn new(volume: f64, neck_radius: f64, neck_length: f64, speed_of_sound: f64) -> Self {
        Self {
            volume,
            neck_radius,
            neck_length,
            speed_of_sound,
        }
    }

    pub fn from_device(device: &UrnDevice, speed_of_sound: f64) -> Self {
        Self {
            volume: device.urn_volume,
            neck_radius: device.neck_radius,
            neck_length: device.neck_length,
            speed_of_sound,
        }
    }

    pub fn resonance_frequency(&self) -> f64 {
        let neck_area = PI * self.neck_radius.powi(2);
        let effective_length = self.neck_length + 1.7 * self.neck_radius;
        let denominator = 2.0 * PI * (self.volume * effective_length / neck_area).sqrt();
        self.speed_of_sound / denominator
    }

    pub fn gain_at_frequency(&self, frequency: f64) -> f64 {
        let f0 = self.resonance_frequency();
        let q = self.quality_factor();
        let x = frequency / f0;
        let numerator = q;
        let denominator = ((1.0 - x.powi(2)).powi(2) + (x / q).powi(2)).sqrt();
        numerator / denominator
    }

    pub fn gain_db(&self, frequency: f64) -> f64 {
        let gain = self.gain_at_frequency(frequency);
        20.0 * gain.log10()
    }

    pub fn quality_factor(&self) -> f64 {
        let neck_area = PI * self.neck_radius.powi(2);
        let effective_length = self.neck_length + 1.7 * self.neck_radius;
        let viscous_loss = 2.0 * (2.0 * PI * frequency() * 1.5e-5).sqrt() / self.neck_radius;
        let _ = viscous_loss;
        let q_geometric = (self.volume.sqrt() * neck_area)
            / (effective_length.powi(2) * 2.0_f64.sqrt() * PI);
        q_geometric.min(80.0).max(5.0)
    }

    pub fn finite_element_gain_correction(&self, frequency: f64, medium_density: f64) -> f64 {
        let base_gain = self.gain_db(frequency);
        let density_correction = (medium_density / 1600.0).ln() * 2.5;
        let f0 = self.resonance_frequency();
        let ratio = frequency / f0;
        let fe_correction = if ratio < 0.5 {
            -1.5 * (0.5 - ratio)
        } else if ratio > 1.5 {
            -2.0 * (ratio - 1.5)
        } else {
            0.5 * (-(ratio - 1.0).powi(2) / 0.3).exp()
        };
        base_gain + density_correction + fe_correction
    }
}

fn frequency() -> f64 {
    200.0
}

pub struct AcousticAnalyzer {
    speed_of_sound: f64,
    drift_warning_threshold: f64,
    drift_critical_threshold: f64,
}

impl AcousticAnalyzer {
    pub fn new(speed_of_sound: f64, drift_warning_percent: f64, drift_critical_percent: f64) -> Self {
        Self {
            speed_of_sound,
            drift_warning_threshold: drift_warning_percent / 100.0,
            drift_critical_threshold: drift_critical_percent / 100.0,
        }
    }

    pub fn analyze(
        &self,
        reading: &SensorReading,
        device: &UrnDevice,
    ) -> ResonanceAnalysisResult {
        let resonator = HelmholtzResonator::from_device(device, self.speed_of_sound);

        let theoretical_freq = resonator.resonance_frequency();
        let measured_freq = reading.resonance_frequency;
        let drift = measured_freq - theoretical_freq;
        let drift_percent = (drift / theoretical_freq).abs() * 100.0;

        let gain_db = resonator.finite_element_gain_correction(
            measured_freq,
            reading.medium_density,
        );

        let q_factor = resonator.quality_factor();
        let is_anomaly = drift_percent > self.drift_warning_threshold * 100.0;

        ResonanceAnalysisResult {
            timestamp: Utc::now(),
            device_id: reading.device_id,
            measured_resonance_freq: measured_freq,
            theoretical_resonance_freq: theoretical_freq,
            gain_db,
            quality_factor: q_factor,
            frequency_drift: drift,
            drift_percent,
            is_anomaly,
        }
    }

    pub fn is_critical_drift(&self, drift_percent: f64) -> bool {
        drift_percent > self.drift_critical_threshold * 100.0
    }

    pub fn is_warning_drift(&self, drift_percent: f64) -> bool {
        drift_percent > self.drift_warning_threshold * 100.0
    }
}

pub struct WavePropagation {
    pub speed: f64,
    pub attenuation_coeff: f64,
}

impl WavePropagation {
    pub fn new(speed: f64, attenuation_coeff: f64) -> Self {
        Self { speed, attenuation_coeff }
    }

    pub fn travel_time(&self, distance: f64) -> f64 {
        distance / self.speed
    }

    pub fn amplitude_attenuation(&self, initial_amplitude: f64, distance: f64) -> f64 {
        let geometric_spreading = 1.0 / distance.max(1.0);
        let material_attenuation = (-self.attenuation_coeff * distance).exp();
        initial_amplitude * geometric_spreading * material_attenuation
    }

    pub fn phase_shift(&self, distance: f64, frequency: f64) -> f64 {
        let wavelength = self.speed / frequency;
        2.0 * PI * (distance % wavelength) / wavelength
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_helmholtz_resonance() {
        let resonator = HelmholtzResonator::new(0.05, 0.05, 0.1, 343.0);
        let f0 = resonator.resonance_frequency();
        assert!(f0 > 50.0 && f0 < 500.0);
    }

    #[test]
    fn test_gain_calculation() {
        let resonator = HelmholtzResonator::new(0.05, 0.05, 0.1, 343.0);
        let f0 = resonator.resonance_frequency();
        let gain = resonator.gain_at_frequency(f0);
        assert!(gain > 1.0);
    }
}
