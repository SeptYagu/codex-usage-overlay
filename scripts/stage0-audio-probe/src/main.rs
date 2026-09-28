use rodio::{Decoder, DeviceSinkBuilder, Player, Source};
use std::{fs::File, path::Path, time::{Duration, Instant}};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let directory = args.next().ok_or("missing sample directory")?;
    let playback = args.any(|arg| arg == "--playback");
    for name in ["short.mp3", "short.aac", "short.m4a", "short.wav", "long.wav"] {
        let path = Path::new(&directory).join(name);
        let source = Decoder::try_from(File::open(&path)?)?;
        let decoded_samples = source.take_duration(Duration::from_secs(10)).count();
        if decoded_samples == 0 {
            return Err(format!("{name}: no decoded samples").into());
        }
        println!("{name}: {decoded_samples} decoded samples within 10 s cap");
    }
    if playback {
        let sink = DeviceSinkBuilder::open_default_sink()?;
        for name in ["short.mp3", "short.aac", "short.m4a", "short.wav", "long.wav"] {
            let player = Player::connect_new(sink.mixer());
            let source = Decoder::try_from(File::open(Path::new(&directory).join(name))?)?
                .take_duration(Duration::from_secs(10));
            let start = Instant::now();
            player.append(source);
            player.sleep_until_end();
            println!("{name}: playback elapsed {:.3}s", start.elapsed().as_secs_f64());
            player.stop();
        }
    }
    Ok(())
}
