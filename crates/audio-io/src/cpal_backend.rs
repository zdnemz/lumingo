//! The real audio backend, on `cpal` 0.18 (WASAPI on Windows).
//!
//! UNVERIFIED. This file has been compile-checked and nothing more. It has never
//! opened a device, because the build container has no audio hardware. What the
//! owner must check on Windows with a microphone and speakers is listed in the
//! crate README.
//!
//! Everything device-independent (ring buffer, conversion, gate, playback queue,
//! registry, session) is tested against the fake backend instead.

use std::str::FromStr;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{ErrorKind, FromSample, SampleFormat, SizedSample, StreamConfig};

use crate::device::{
    AudioBackend, AudioStream, DeviceError, DeviceId, DeviceInfo, Direction, InputCallback,
    OutputCallback, StreamErrorCallback,
};
use crate::format::StreamFormat;

/// Capacity reserved for the sample-format conversion buffers, in samples. It is
/// 100 ms of 8-channel 96 kHz audio, far above any normal callback size, so the
/// callbacks do not allocate.
const SCRATCH_SAMPLES: usize = 96_000 * 8 / 10;

/// The system audio API through `cpal`'s default host.
#[derive(Debug, Default, Clone, Copy)]
pub struct CpalBackend;

impl CpalBackend {
    pub fn new() -> Self {
        Self
    }
}

struct CpalStream {
    _stream: cpal::Stream,
}

// Dropping the inner stream stops it. On WASAPI cpal's `Drop for Stream` sends the
// terminate command and joins the render or capture thread.
impl AudioStream for CpalStream {}

fn backend_error(error: &cpal::Error) -> DeviceError {
    DeviceError::Backend(error.to_string())
}

fn find_device(info: &DeviceInfo) -> Result<cpal::Device, DeviceError> {
    let id = cpal::DeviceId::from_str(&info.id.0).map_err(|e| backend_error(&e))?;
    cpal::default_host()
        .device_by_id(&id)
        .ok_or_else(|| DeviceError::NotFound {
            direction: info.direction,
            name: info.name.clone(),
        })
}

fn describe(
    device: &cpal::Device,
    direction: Direction,
    default_id: Option<&cpal::DeviceId>,
) -> Option<DeviceInfo> {
    let id = device.id().ok()?;
    let name = device.description().ok()?.name().to_owned();
    let config = match direction {
        Direction::Input => device.default_input_config(),
        Direction::Output => device.default_output_config(),
    }
    .ok()?;
    Some(DeviceInfo {
        is_default: default_id == Some(&id),
        id: DeviceId(id.to_string()),
        name,
        direction,
        format: StreamFormat::new(config.sample_rate(), config.channels()),
    })
}

fn open_error(info: &DeviceInfo, error: &cpal::Error) -> DeviceError {
    match error.kind() {
        ErrorKind::DeviceNotAvailable => DeviceError::NotFound {
            direction: info.direction,
            name: info.name.clone(),
        },
        ErrorKind::UnsupportedConfig
        | ErrorKind::UnsupportedOperation
        | ErrorKind::InvalidInput
        | ErrorKind::DeviceBusy
        | ErrorKind::PermissionDenied => DeviceError::Unsupported {
            direction: info.direction,
            name: info.name.clone(),
            detail: error.to_string(),
        },
        _ => backend_error(error),
    }
}

/// Turns a running stream's error into the crate's error, or `None` for
/// notices that need no action.
fn running_error(direction: Direction, name: &str, error: &cpal::Error) -> Option<DeviceError> {
    match error.kind() {
        // The device is gone, or its configuration changed under the stream and
        // it must be rebuilt: both are recovered by reopening.
        ErrorKind::DeviceNotAvailable | ErrorKind::StreamInvalidated => Some(DeviceError::Lost {
            direction,
            name: name.to_owned(),
        }),
        // Glitch notices and automatic rerouting: the stream keeps running.
        ErrorKind::Xrun | ErrorKind::DeviceChanged | ErrorKind::RealtimeDenied => {
            tracing::debug!(%error, "audio stream notice");
            None
        }
        _ => Some(backend_error(error)),
    }
}

fn stream_config(info: &DeviceInfo) -> StreamConfig {
    StreamConfig {
        channels: info.format.channels,
        sample_rate: info.format.sample_rate,
        buffer_size: cpal::BufferSize::Default,
    }
}

fn error_forwarder(
    info: &DeviceInfo,
    on_error: StreamErrorCallback,
) -> impl FnMut(cpal::Error) + Send + 'static {
    let (direction, name) = (info.direction, info.name.clone());
    move |error| {
        if let Some(mapped) = running_error(direction, &name, &error) {
            on_error(mapped);
        }
    }
}

fn build_input<T>(
    device: &cpal::Device,
    config: StreamConfig,
    mut callback: InputCallback,
    on_error: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample + Send + 'static,
    f32: FromSample<T>,
{
    let mut scratch: Vec<f32> = Vec::with_capacity(SCRATCH_SAMPLES);
    device.build_input_stream::<T, _, _>(
        config,
        move |data: &[T], _| {
            scratch.clear();
            scratch.extend(data.iter().map(|sample| f32::from_sample_(*sample)));
            callback(&scratch);
        },
        on_error,
        None,
    )
}

fn build_output<T>(
    device: &cpal::Device,
    config: StreamConfig,
    mut callback: OutputCallback,
    on_error: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    let mut scratch: Vec<f32> = Vec::with_capacity(SCRATCH_SAMPLES);
    device.build_output_stream::<T, _, _>(
        config,
        move |data: &mut [T], _| {
            scratch.clear();
            scratch.resize(data.len(), 0.0);
            callback(&mut scratch);
            for (out, sample) in data.iter_mut().zip(&scratch) {
                *out = T::from_sample_(*sample);
            }
        },
        on_error,
        None,
    )
}

fn unsupported_format(info: &DeviceInfo, format: SampleFormat) -> DeviceError {
    DeviceError::Unsupported {
        direction: info.direction,
        name: info.name.clone(),
        detail: format!("the sample format {format} is not supported"),
    }
}

impl AudioBackend for CpalBackend {
    fn devices(&self, direction: Direction) -> Result<Vec<DeviceInfo>, DeviceError> {
        let host = cpal::default_host();
        let default = match direction {
            Direction::Input => host.default_input_device(),
            Direction::Output => host.default_output_device(),
        };
        let default_id = default.and_then(|d| d.id().ok());
        let devices = match direction {
            Direction::Input => host.input_devices(),
            Direction::Output => host.output_devices(),
        }
        .map_err(|e| backend_error(&e))?;
        Ok(devices
            .filter_map(|device| describe(&device, direction, default_id.as_ref()))
            .collect())
    }

    fn open_input(
        &self,
        info: &DeviceInfo,
        callback: InputCallback,
        on_error: StreamErrorCallback,
    ) -> Result<Box<dyn AudioStream>, DeviceError> {
        let device = find_device(info)?;
        let format = device
            .default_input_config()
            .map_err(|e| open_error(info, &e))?
            .sample_format();
        let config = stream_config(info);
        let errors = error_forwarder(info, on_error);
        let stream = match format {
            SampleFormat::F32 => build_input::<f32>(&device, config, callback, errors),
            SampleFormat::I16 => build_input::<i16>(&device, config, callback, errors),
            SampleFormat::U16 => build_input::<u16>(&device, config, callback, errors),
            SampleFormat::I32 => build_input::<i32>(&device, config, callback, errors),
            other => return Err(unsupported_format(info, other)),
        }
        .map_err(|e| open_error(info, &e))?;
        stream.play().map_err(|e| open_error(info, &e))?;
        Ok(Box::new(CpalStream { _stream: stream }))
    }

    fn open_output(
        &self,
        info: &DeviceInfo,
        callback: OutputCallback,
        on_error: StreamErrorCallback,
    ) -> Result<Box<dyn AudioStream>, DeviceError> {
        let device = find_device(info)?;
        let format = device
            .default_output_config()
            .map_err(|e| open_error(info, &e))?
            .sample_format();
        let config = stream_config(info);
        let errors = error_forwarder(info, on_error);
        let stream = match format {
            SampleFormat::F32 => build_output::<f32>(&device, config, callback, errors),
            SampleFormat::I16 => build_output::<i16>(&device, config, callback, errors),
            SampleFormat::U16 => build_output::<u16>(&device, config, callback, errors),
            SampleFormat::I32 => build_output::<i32>(&device, config, callback, errors),
            other => return Err(unsupported_format(info, other)),
        }
        .map_err(|e| open_error(info, &e))?;
        stream.play().map_err(|e| open_error(info, &e))?;
        Ok(Box::new(CpalStream { _stream: stream }))
    }
}
