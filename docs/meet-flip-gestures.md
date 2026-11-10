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
