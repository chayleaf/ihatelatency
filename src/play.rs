use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::traits::{Consumer, Observer};

use crate::RingCons;

pub fn main(
    mut cons: RingCons,
    buffer_samples: usize,
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
    let stream = device.build_output_stream(
        &config,
        move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
            data.fill(0);
            match cons.wait_occupied(1) {
                Ok(()) => {}
                Err(err) => match err {
                    ringbuf_blocking::WaitError::Closed => {
                        log::error!("ringbuf closed");
                        std::process::exit(1);
                    }
                    ringbuf_blocking::WaitError::TimedOut => return,
                },
            }
            let mut len = cons.occupied_len();
            while len > 0 {
                len = len.saturating_sub(cons.pop_slice(unsafe {
                    std::slice::from_raw_parts_mut(data.as_mut_ptr().cast(), data.len() * 2)
                }));
                if len < data.len() + buffer_samples * std::mem::size_of::<i16>() {
                    break;
                }
            }
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
