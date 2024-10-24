# ihatelatency

This is a project I made for streaming audio from my PC to my phone,
because other solutions were kinda annoying to deal with.

Currently, playback is handled via `cpal` for maximum compatibility, but
recording is handled via `pipewire` for minimum latency.

Usage:

```shell
# on the device to stream audio from (server)
pactl load-module module-null-sink sink_name=remote
ihatelatency -l -a <listen_address> record -n remote
# on the device to stream audio to (client)
ihatelatency -a <server_address> play
```

Roles can be switched, the recording device is allowed to be the one to
connect to the playing server. Optionally, the `-u` flag may be added to
use UDP instead of TCP. Note that for UDP the playback device must be
the server.

By default, the playback buffersize is autoadjusted based on how stable
the network is. This is pretty conservative, the buffer is adjusted so
there are barely any xruns. The `-s` flag for the `play` command allows
you to choose the buffer size yourself, picking the right balance
between latency and xruns. If you set the env var `RUST_LOG=debug`, the
program will print a log message whenever an xrun occurs, helping you
decide on the perfect buffer size.

Currently, `s16le`, `48000`, stereo is assumed, but it should be fairly
trivial to add support to sample rate selection (or sending it on
connection, except for UDP).

Also, currently the server can only handle one client at a time since I
don't have a need for streaming audio to multiple devices (and receiving
audio from multiple devices is its own can of worms).

You may actually use other programs as players, like this:

```shell
# udp (most of these flags aren't required but may or may not decrease latency)
ffplay -nodisp -ac 2 -ar 48000 -analyzeduration 0 -probesize 32 -fflags nobuffer -flags low_delay -fflags discardcorrupt -f s16le -i "udp://<listen_address>?listen=1"

# udp
nc -u -l 0.0.0.0 <listen_port> | mpv --demuxer=rawaudio --no-cache --untimed --no-demuxer-thread --demuxer-rawaudio-rate=48000 -
```

Or as recorders, like this:

```shell
# udp
ffmpeg -f pulse -i <sink_name> -ac 2 -f s16le udp://<server_address>

# tcp (please don't send pcm via udp netcat)
parec --format=s16le -d <sink_name> --latency-msec=1 --rate=48000 | nc <server_address> <server_port>
```

Nonetheless, I guarantee that my program is at least as good as other
programs in terms of latency, the only other thing you can tune is
network settings, or sound server settings ([here's a post explaining how
to do it for PulseAudio](https://juho.tykkala.fi/Pulseaudio-and-latency))
