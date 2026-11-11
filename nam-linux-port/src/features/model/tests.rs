use super::NamModel;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new(extension: &str, bytes: &[u8]) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "nam-core-test-{}-{}.{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            extension
        ));
        fs::write(&path, bytes).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn linear(weights: &str, channels: usize, rate: &str) -> Fixture {
    Fixture::new(
        "nam",
        format!(
            r#"{{"version":"0.5.4","architecture":"Linear",
        "config":{{"receptive_field":1,"bias":false,"in_channels":1,"out_channels":{channels}}},
        "weights":[{weights}]{rate}}}"#
        )
        .as_bytes(),
    )
}

#[test]
fn official_linear_inference_and_mono_duplication() {
    for gain in [1.0, 0.25, -2.0] {
        let file = linear(&gain.to_string(), 1, ",\"sample_rate\":48000");
        let mut model = NamModel::load(&file.0, 48000.0, 64).unwrap();
        assert_eq!(model.input_channels(), 1);
        assert_eq!(model.output_channels(), 1);
        assert_eq!(model.expected_sample_rate(), Some(48000.0));
        let input = [1.0, -0.5, 0.0, 0.125];
        let (mut left, mut right) = ([0.0; 4], [0.0; 4]);
        for _ in 0..100 {
            model.process(&input, &mut left, &mut right).unwrap();
            assert_eq!(left, input.map(|x| x * gain));
            assert_eq!(right, left);
        }
        model.reset().unwrap();
        model.process(&input, &mut left, &mut right).unwrap();
        assert_eq!(left, input.map(|x| x * gain));
    }
}

#[test]
fn official_wavenet_inference_handles_variable_blocks_and_duplicates_mono() {
    // NAM/wavenet/model.cpp consumes rechannel, conv (weight/bias), input mixin,
    // layer1x1 (weight/bias), head rechannel, then head scale.
    // This one-channel WaveNet computes 0.5 * ReLU(input).
    let file = Fixture::new(
        "nam",
        br#"{"version":"0.5.4","architecture":"WaveNet","sample_rate":48000,
        "config":{"layers":[{"input_size":1,"condition_size":1,"channels":1,
        "head_size":1,"head_bias":false,"kernel_size":1,"dilations":[1],
        "activation":"ReLU","gated":false}],"head":null,"head_scale":0.5},
        "weights":[1,1,0,0,0,0,1,0.5]}"#,
    );
    let mut model = NamModel::load(&file.0, 48000.0, 8).unwrap();
    assert_eq!(model.input_channels(), 1);
    assert_eq!(model.output_channels(), 1);
    let input = [-1.0_f32, 0.5, 1.0, 0.0, -0.25, 0.125, 2.0, -2.0];
    let (mut left, mut right) = ([f32::NAN; 8], [f32::NAN; 8]);
    for frames in [8, 1, 5, 2, 8] {
        left.fill(f32::NAN);
        right.fill(f32::NAN);
        model
            .process(&input[..frames], &mut left[..frames], &mut right[..frames])
            .unwrap();
        for i in 0..frames {
            let expected = 0.5 * input[i].max(0.0);
            assert!(left[i].is_finite(), "block {frames}, sample {i}");
            assert_eq!(left[i], expected, "block {frames}, sample {i}");
            assert_eq!(right[i], left[i], "mono block {frames}, sample {i}");
        }
    }
}

#[test]
fn invalid_file_and_invalid_blocks_fail() {
    let file = Fixture::new("nam", b"not json");
    assert!(NamModel::load(&file.0, 48000.0, 64).is_err());
    assert!(NamModel::load(file.0.with_extension("missing"), 48000.0, 64).is_err());
    let file = linear("1", 1, "");
    assert!(NamModel::load(&file.0, f64::NAN, 64).is_err());
    assert!(NamModel::load(&file.0, 48000.0, 0).is_err());
    let mut model = NamModel::load(&file.0, 48000.0, 4).unwrap();
    assert_eq!(model.expected_sample_rate(), None);
    assert!(model
        .process(&[0.0; 5], &mut [0.0; 5], &mut [0.0; 5])
        .is_err());
    assert!(model
        .process(&[0.0; 4], &mut [0.0; 3], &mut [0.0; 4])
        .is_err());
    model.process(&[], &mut [], &mut []).unwrap();
}

#[test]
fn neural_model_rate_mismatch_is_not_silently_retuned() {
    // One cell: eight input/recurrent weights, four biases, h/c state, head weight/bias.
    let file = Fixture::new(
        "nam",
        br#"{"version":"0.5.4","architecture":"LSTM",
        "config":{"input_size":1,"hidden_size":1,"num_layers":1},
        "weights":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,0],"sample_rate":48000}"#,
    );
    let error = NamModel::load(&file.0, 44100.0, 64).err().unwrap();
    assert!(
        error.contains("48000") && error.contains("44100"),
        "{error}"
    );
    let mut model = NamModel::load(&file.0, 48000.0, 64).unwrap();
    assert!(!model.supports_arbitrary_sample_rate());
    model
        .process(&[0.0; 4], &mut [0.0; 4], &mut [0.0; 4])
        .unwrap();
}

fn wav(channels: u16) -> Fixture {
    let mut bytes = Vec::new();
    let size = u32::from(channels) * 4;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + size).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&3u16.to_le_bytes());
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&48000u32.to_le_bytes());
    bytes.extend_from_slice(&(48000 * u32::from(channels) * 4).to_le_bytes());
    bytes.extend_from_slice(&(channels * 4).to_le_bytes());
    bytes.extend_from_slice(&32u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&size.to_le_bytes());
    bytes.extend_from_slice(&0.5f32.to_le_bytes());
    if channels == 2 {
        bytes.extend_from_slice(&(-0.25f32).to_le_bytes());
    }
    Fixture::new("WaV", &bytes)
}

#[test]
fn mono_and_stereo_wav_use_real_core_convolution() {
    for channels in [1, 2] {
        let file = wav(channels);
        let mut model = NamModel::load(&file.0, 48000.0, 8).unwrap();
        assert_eq!(model.output_channels(), channels as usize);
        let (mut left, mut right) = ([0.0; 3], [0.0; 3]);
        model
            .process(&[1.0, 0.5, -1.0], &mut left, &mut right)
            .unwrap();
        assert_eq!(left, [0.5, 0.25, -0.5]);
        assert_eq!(
            right,
            if channels == 1 {
                left
            } else {
                [-0.25, -0.125, 0.25]
            }
        );
        let resampled = NamModel::load(&file.0, 96000.0, 8).unwrap();
        assert!(resampled.supports_arbitrary_sample_rate());
        assert_eq!(resampled.expected_sample_rate(), Some(48000.0));
        assert_eq!(resampled.sample_rate(), 96000.0);
    }
}

#[test]
fn rejects_unsupported_channels() {
    let file = linear("1,1,1", 3, ",\"sample_rate\":48000");
    let error = NamModel::load(&file.0, 48000.0, 8).err().unwrap();
    assert!(error.contains("mono/stereo"), "{error}");
    fn assert_send<T: Send>() {}
    assert_send::<NamModel>();
}
