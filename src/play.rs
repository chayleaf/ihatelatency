use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::traits::{Consumer, Observer};

use crate::RingCons;

pub fn main(
    mut cons: RingCons,
    buffer_bytes: Option<usize>,
    device_name: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let host = cpal::default_host();
    let device = if let Some(name) = device_name {
        host.output_devices()
            .unwrap()
            .find(|dev| {
                let dev = dev.name();
                log::info!("trying device {dev:?}");
                matches!(dev, Ok(dev) if dev == name)
            })
            .expect("device not found")
    } else {
        host.default_output_device().unwrap()
    };
    let mut supported_configs_range = device.supported_output_configs().unwrap();
    let supported_config = supported_configs_range
        .find(|cfg| {
            cfg.min_sample_rate().0 <= 48000
                && cfg.max_sample_rate().0 >= 48000
                && cfg.channels() == 2
                && cfg.sample_format().is_int()
                && cfg.sample_format().sample_size() == 2
        })
        .unwrap()
        .with_sample_rate(cpal::SampleRate(48000));
    let config = supported_config.into();
    cons.set_timeout(Some(Duration::from_millis(10)));
    let mut auto_buffer_size = 0.0f64;
    let mut last_rx = None;
    let stream = device.build_output_stream(
        &config,
        move |data: &mut [i16], info: &cpal::OutputCallbackInfo| {
            data.fill(0);
            // type annotation
            if false {
                last_rx = Some(info.timestamp().callback);
            }
            match cons.wait_occupied(1) {
                Ok(()) => {}
                Err(err) => match err {
                    ringbuf_blocking::WaitError::Closed => {
                        log::error!("ringbuf closed");
                        std::process::exit(1);
                    }
                    ringbuf_blocking::WaitError::TimedOut => {
                        // consider a timeout to have no xruns
                        auto_buffer_size = (auto_buffer_size - 0.01).max(0.0);
                        return;
                    }
                },
            }
            last_rx = Some(info.timestamp().callback);
            cons.skip(
                cons.occupied_len()
                    .saturating_sub(data.len() * 2)
                    .saturating_sub(
                        buffer_bytes.unwrap_or_else(|| auto_buffer_size.ceil() as usize * 2),
                    ),
            );
            if buffer_bytes.is_none() {
                if cons.occupied_len() < data.len() * 2 {
                    if auto_buffer_size == 0.0 {
                        auto_buffer_size += 0.1;
                    } else {
                        let old_buf_size = auto_buffer_size as usize;
                        auto_buffer_size = (auto_buffer_size
                            + (data.len() / 10 - cons.occupied_len() / 20) as f64)
                            .min(100000.0);
                        log::debug!(
                            "xrun ({} samples, buffer size {}, changing buffer size to {})",
                            data.len() - cons.occupied_len() / 2,
                            old_buf_size,
                            auto_buffer_size as usize
                        );
                    }
                } else {
                    auto_buffer_size = (auto_buffer_size - 0.01).max(0.0);
                }
            } else if cons.occupied_len() < data.len() * 2 {
                log::debug!("xrun ({} samples)", data.len() - cons.occupied_len() / 2);
            }
            cons.pop_slice(unsafe {
                std::slice::from_raw_parts_mut(data.as_mut_ptr().cast(), data.len() * 2)
            });
        },
        move |err| {
            log::error!("cpal: {err}");
        },
        None, // blocking
    )?;
    stream.play()?;
    std::thread::sleep(Duration::MAX);
    Ok(())
}
