# OBSBOT Meet Flip gesture controls

Verified on a connected OBSBOT Meet Flip (USB `3564:ff06`) on 2026-11-10.
No video capture, preview, driver detachment, reset or firmware update was used.

## Transport

The VideoControl interface exposes extension unit ID 2, GUID
`{9A1E7291-6843-4683-6D92-39BC7906EE49}`.
Use Linux `UVCIOC_CTRL_QUERY` on the camera's capture node, with selector 6.
The descriptor and `GET_LEN` report 60 bytes; `GET_INFO` reports `0x03` (GET/SET).

## Read current state

Send `GET_CUR` (`0x81`), unit 2, selector 6, length 60.
On the tested firmware, byte 0 reports a 46-byte status structure. The gesture
word at offsets 38–39 is little-endian:

| Bits | Meaning |
| --- | --- |
| 0 | Framing gesture enabled |
| 1–2 | Framing mode |
| 3 | Zoom gesture enabled |
| 4–15 | Gesture zoom ratio, percent (200 = 2×) |

These fields match the SDK's Meet 2 / Meet SE status layout. They were confirmed
on the Flip by changing each enable bit independently and reading it back.
Other models and firmware versions require their own verification.

## Set gestures while preserving other fields

Send `SET_CUR` (`0x01`), unit 2, selector 6, length 60. Build this payload and pad
with zeros to 60 bytes:

| Offset | Size | Value |
| --- | --- | --- |
| 0 | 2 | `0x0520`, little-endian (`20 05`) |
| 2 | 1 | Framing gesture enabled: 0/1 |
| 3 | 1 | Framing mode, preserved from the current status |
| 4 | 1 | Zoom gesture enabled: 0/1 |
| 5 | 2 | Gesture zoom ratio, little-endian, preserved from current status |
| 7–59 | 53 | Zero padding |

This direct status/control message has no framed-protocol checksum.
Do not confuse it with the separate framed V3 command transport on selector 2.
Read state before each write and confirm the complete gesture word afterwards.

## Observed result

Initial gesture word: `0x0c8d` (framing on, mode 2, zoom on, ratio 200).
After disabling framing: `0x0c8c`.
After disabling zoom as well: `0x0c84`.
Each write changed only status byte 38. Readback confirmed both gestures off;
framing mode and gesture zoom ratio were preserved. Later status reads confirmed
the flags remained off during the current USB session.

## Evidence and limitations

The transport and payload were reconstructed by static inspection of
`Device::cameraGetCameraStatusU`, `Device::cameraSetGestureControlU`,
`DevicePrivate::uvcExtGet`, `DevicePrivate::uvcExtSet`, and `UvcProtocol::setData`
in the [public Linux SDK v1.0.2](https://github.com/aaronsb/obsbot-camera-control/tree/main/sdk/v1.0.2).
The binary was not executed and is not included in this report.

Confirmed: camera status readback and preservation of adjacent settings.
Not yet tested: physical gesture behavior, persistence across USB reconnect or
power loss, and support on other firmware versions. No automatic reapply loop
has been installed.

## Active auto framing (separate from gestures)

Status byte 24 is the active AI mode: 0 = standard mode, 1 = group framing,
2 = single-person framing. The utility displays Off, Group framing or Single-person framing. Mode 6 is a transient switching state; the adapter waits for it to settle.
Turning off framing gestures does not stop auto framing that is already running.

The active switch uses selector 6, `SET_CUR`, 60 bytes:
`16 02 <mode> 00`, followed by zero padding. This was reconstructed from
`Device::cameraSetAiModeU`. The utility writes mode 0 for off, mode 1 for group framing, and mode 2 for
single-person framing. The fourth byte is 0: normal software command source for
off/group, or normal view for single-person framing. It is not a gesture setting.
All three modes have been confirmed by status readback on the Flip.

On the tested Flip, switching off changed active mode byte 24 from 1 to 0,
AI submode byte 28 from 1 to 0, and boot mode byte 32 from 17 to 0. The gesture
word stayed unchanged. Thus this firmware also updates its reported boot mode;
persistence across a physical unplug has not been tested. Auto-framing changes
are verified by status readback independently of the LED color.

## HDR

HDR state is status byte 6 (0 = off, 1 = on). The SDK's `cameraSetWdrR` uses
the same direct extension control for this device family: selector 6,
`SET_CUR`, 60 bytes, starting `01 01 <enabled>`, zero-padded.
This is distinct from the framed WDR command used by some other models.

Native Rust on/off round trips were verified on the Flip; AI mode and gesture
preferences were preserved. The SDK recommends at least three seconds between
HDR switches because sensor reconfiguration takes time. The UI enforces that
interval and the adapter allows up to four seconds for HDR readback to settle.
Firmware may restrict HDR at particular video formats; a failed readback is
reported instead of assuming success. Preview remains optional and is not
started by these controls.
