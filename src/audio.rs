use std::collections::VecDeque;

#[cfg(target_os = "linux")]
#[path = "audio_linux.rs"]
mod platform;
#[cfg(windows)]
#[path = "audio_windows.rs"]
mod platform;

pub use platform::MusicCapture;

const FFT_SIZE: usize = 512;
const SAMPLE_RATE: usize = 16_000;

fn spectrum(samples: &[f32; FFT_SIZE]) -> Vec<u8> {
    let mut real = [0_f32; FFT_SIZE];
    let mut imaginary = [0_f32; FFT_SIZE];
    let bits = FFT_SIZE.trailing_zeros();
    for (index, sample) in samples.iter().enumerate() {
        let reversed = index.reverse_bits() >> (usize::BITS - bits);
        let window =
            0.5 - 0.5 * (std::f32::consts::TAU * index as f32 / (FFT_SIZE - 1) as f32).cos();
        real[reversed] = sample * window;
    }
    let mut width = 2;
    while width <= FFT_SIZE {
        let half = width / 2;
        for offset in (0..FFT_SIZE).step_by(width) {
            for index in 0..half {
                let angle = -std::f32::consts::TAU * index as f32 / width as f32;
                let (sine, cosine) = angle.sin_cos();
                let a = offset + index;
                let b = a + half;
                let re = real[b] * cosine - imaginary[b] * sine;
                let im = real[b] * sine + imaginary[b] * cosine;
                real[b] = real[a] - re;
                imaginary[b] = imaginary[a] - im;
                real[a] += re;
                imaginary[a] += im;
            }
        }
        width *= 2;
    }
    (0..FFT_SIZE / 2)
        .map(|index| {
            let magnitude = (real[index].hypot(imaginary[index]) * 4.0 / FFT_SIZE as f32).max(1e-9);
            let decibels = 20.0 * magnitude.log10();
            ((decibels + 90.0) / 80.0 * 255.0).clamp(0.0, 255.0).round() as u8
        })
        .collect()
}

fn upstream_level(frequencies: &[u8], history: &mut VecDeque<f32>) -> usize {
    let positive: Vec<u8> = frequencies
        .iter()
        .copied()
        .filter(|value| *value > 0)
        .collect();
    if positive.is_empty() {
        return 1;
    }
    let mean = positive.iter().map(|value| f32::from(*value)).sum::<f32>() / positive.len() as f32;
    history.push_back(mean);
    if history.len() > 150 {
        history.pop_front();
    }
    let min = history.iter().copied().fold(f32::INFINITY, f32::min);
    let max = history.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let range = if max == min { 1.0 } else { max - min };
    (((mean - min) / range) * 6.0).floor().clamp(0.0, 6.0) as usize
}

fn upstream_colors(level: usize, color: [u8; 3], brightness: u8) -> [[u8; 3]; 12] {
    let groups: &[&[usize]] = &[&[5], &[4, 6], &[3, 7], &[2, 8], &[1, 9], &[0, 10], &[11]];
    let scale = (0.3 + f32::from(brightness.min(4)) * 0.175) * 0.8;
    let lit = color.map(|channel| (f32::from(channel) * scale).round() as u8);
    let mut leds = [[0; 3]; 12];
    for group in groups.iter().take(level.min(6)) {
        for index in *group {
            leds[*index] = lit;
        }
    }
    leds
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fft_finds_synthetic_frequency_and_silence() {
        let tone = std::array::from_fn(|index| {
            0.25 * (std::f32::consts::TAU * 500.0 * index as f32 / SAMPLE_RATE as f32).sin()
        });
        let bins = spectrum(&tone);
        let maximum = bins
            .iter()
            .copied()
            .enumerate()
            .max_by_key(|(_, value)| *value)
            .unwrap()
            .0;
        assert!((15..=17).contains(&maximum));
        assert!(bins[16] > bins[64]);
        assert!(spectrum(&[0.0; FFT_SIZE]).iter().all(|value| *value == 0));
    }

    #[test]
    fn led_mapping_and_brightness_match_web_sender() {
        let leds = upstream_colors(6, [255, 100, 0], 4);
        assert_eq!(leds[0], [204, 80, 0]);
        assert_eq!(leds[10], [204, 80, 0]);
        assert_eq!(leds[11], [0, 0, 0]);
        let quiet = upstream_colors(1, [255, 0, 0], 0);
        assert_eq!(quiet[5], [61, 0, 0]);
        assert_eq!(quiet.iter().filter(|value| **value != [0, 0, 0]).count(), 1);
    }

    #[test]
    fn amplitude_history_matches_web_bounds() {
        let mut history = VecDeque::new();
        assert_eq!(upstream_level(&[0, 0], &mut history), 1);
        assert_eq!(upstream_level(&[10, 10], &mut history), 0);
        assert_eq!(upstream_level(&[20, 20], &mut history), 6);
        for _ in 0..200 {
            upstream_level(&[15], &mut history);
        }
        assert_eq!(history.len(), 150);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn resolves_default_sink_and_rejects_microphone() {
        let mut graph = serde_json::json!([
            {"type":"PipeWire:Interface:Metadata", "props":{"metadata.name":"default"},
             "metadata":[{"key":"default.audio.source","value":{"name":"mic"}},
                         {"key":"default.audio.sink","value":{"name":"speakers"}}]},
            {"type":"PipeWire:Interface:Node", "info":{"props":{"node.name":"speakers","media.class":"Audio/Sink","object.serial":53,"node.description":"Speakers"}}}
        ]);
        assert_eq!(
            platform::default_sink(&graph).unwrap(),
            ("53".into(), "Speakers".into())
        );
        graph[1]["info"]["props"]["media.class"] = "Audio/Source".into();
        assert!(platform::default_sink(&graph).is_err());
    }
}
