# Standards test fixtures

These files are deterministic protocol fixtures used by
`tests/standards_conformance.rs`. They are not official conformance vectors and
must not be represented as third-party test evidence.

| File | Shape | Purpose |
| --- | --- | --- |
| `jt808_0100_2019.hex` | 0x0100 registration body | manufacturer/model fields |
| `jt808_0107_2019.hex` | 0x0107 terminal attributes body | attribute/raw field |
| `jt808_0200_mandatory.hex` | 0x0200 basic location body | lat/lon parsing |
| `jt808_8203.hex` | 0x8203 alarm acknowledgement body | short control body |
| `jt809_login.hex` | 0x1001 login frame | CRC/frame/login body |
| `gb32960_realtime.hex` | 0x02 realtime frame | header/VIN/data length |
| `jt1078_media.hex` | 30-byte media header + I-frame body | SIM/channel/keyframe/length |

Rules:

- Keep fixtures small and readable.
- A fixture change must update the matching conformance test.
- Do not add real customer/vehicle/terminal captures without redaction and
  authorization.
- Official GB/T 32960/JT/T 808/JT/T 809/JT/T 1078 conformance requires
  licensed test suites and a qualified lab.
