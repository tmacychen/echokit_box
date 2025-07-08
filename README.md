# Setup the EchoKit device

## Build espflash

Assume that you [installed the Rust compiler](https://www.rust-lang.org/tools/install) on your computer.

```
cargo install cargo-espflash espflash ldproxy
```

## Get firmware

Get a pre-compiled binary version of the firmware.

```
curl -LO https://echokit.dev/firmware/echokit-box
```

To build the `echokit-box` firmware file from source, you can do the following.

```
cargo build --release
```

## Upload firmware

You MUST connect the computer to the SLAVE USB port on the device. Allow the computer to accept connection from the device. The detected USB serial port must be `JTAG`. IT CANNOT be `USB Single`.

```
espflash flash --monitor --flash-size 16mb echokit-box
```

The response is as follows.

```
[2025-04-28T16:51:43Z INFO ] Detected 2 serial ports
[2025-04-28T16:51:43Z INFO ] Ports which match a known common dev board are highlighted
[2025-04-28T16:51:43Z INFO ] Please select a port
✔ Remember this serial port for future use? · no
[2025-04-28T16:52:00Z INFO ] Serial port: '/dev/cu.usbmodem2101'
[2025-04-28T16:52:00Z INFO ] Connecting...
[2025-04-28T16:52:00Z INFO ] Using flash stub
Chip type:         esp32s3 (revision v0.2)
Crystal frequency: 40 MHz
Flash size:        8MB
Features:          WiFi, BLE
... ...
I (705) boot: Loaded app from partition at offset 0x10000
I (705) boot: Disabling RNG early entropy source...
I (716) cpu_start: Multicore app
```

## Reset the device

Reset the device (simulate the RST button or power up).

```
espflash reset
```

Delete the existing firmware if needed.

```
espflash erase-flash
```

## Next steps

You will need to configure and start up an [EchoKit server](https://github.com/second-state/echokit_server), and then configure your device to connect to the server in order for the EchoKit device to be fully functional.





