use alsa::pcm::*;
use alsa::{Direction, ValueOr};
use ringbuf::traits::Producer;

use crate::RingProd;

pub fn main(device_name: String, mut prod: RingProd) -> Result<(), Box<dyn std::error::Error>> {
    let pcm = PCM::new(&device_name, Direction::Playback, false)?;
    {
        let hwp = HwParams::any(&pcm)?;
        hwp.set_channels(2)?;
        hwp.set_rate(48000, ValueOr::Nearest)?;
        hwp.set_format(Format::S16LE)?;
        hwp.set_access(Access::RWInterleaved)?;
        pcm.hw_params(&hwp)?;
    }
    pcm.start()?;
    let io = pcm.io_i16()?;
    let mut buf = [0i16; 8192];
    loop {
        let size = io.readi(&mut buf)?;
        let size = size * 2;
        let mut pushed = 0;
        println!(
            "avg {} len {size}",
            buf.iter().map(|x| isize::from(*x)).sum::<isize>() / size as isize / 2
        );
        let buf = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast(), buf.len() / 2) };
        while pushed < size {
            match prod.wait_vacant(size - pushed) {
                Ok(()) => {}
                Err(err) => match err {
                    ringbuf_blocking::WaitError::TimedOut => continue,
                    ringbuf_blocking::WaitError::Closed => panic!("ringbuf closed"),
                },
            }
            pushed += prod.push_slice(buf);
        }
    }
}
