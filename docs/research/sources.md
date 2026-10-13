# Sources

The current ableton.com site documents Live 12 and must not define the target. The Live 10
URLs (`/en/live-manual/10/...`) redirect to themselves without a session (redirect loop), and
`help.ableton.com` returned HTTP 403 to automated fetches. The Live 10 Reference Manual was
therefore read from Internet Archive snapshots of `https://www.ableton.com/en/manual/<chapter>/`
taken in 2019, whose pages identify themselves as "Ableton Reference Manual Version 10".

Snapshots were retrieved on 2026-10-13 with
`http://web.archive.org/web/2019id_/https://www.ableton.com/en/manual/<chapter>/`.
Copies are not committed (copyrighted); SHA-256 of the retrieved HTML:

| Chapter | Manual section | SHA-256 |
| --- | --- | --- |
| launching-clips | 13 Launching Clips | `2b8d776065c919469b7cf72cddfddda6692943613673a0f10cf5c3c84c3f48f2` |
| session-view | 7 Session View | `580337ed4325aafcca920957781358f7b8a913469e291c5a0fd17cdf29b3ee38` |
| arrangement-view | 6 Arrangement View | `42e8e592a8e159a9c279b9317876251f0ff0dcad6d9ccc49cf02f7420c04de92` |
| recording-new-clips | 16 Recording New Clips | `8ba524bcdaa3f58abaace20396eb507eff32de9c703a8a39201a3a915879d023` |
| clip-view | 8 Clip View | `dae46509b29b4463cada3b721093cd447eebfe79a51e82fd321a37801df07521` |
| mixing | 15 Mixing | `695ed009c611ac5be5e42dab3545272baad3322231dc2e437a07b0e8f42fe125` |
| editing-midi-notes-and-velocities | 10 MIDI editing | `3df3a5e9d9699528a37703123ed59813c477b1cd3739fe8a87fadcf20558e29f` |
| automation-and-editing-envelopes | 19 Automation | `195467c684045ebfc47835d85ae1acddda7f9d998454976f734a37e179cf4d09` |
| routing-and-i-o | 14 Routing and I/O | `2fbf52cb0a294ac69f20e71515992d48acf8c84666506a3ab33e8d3d822cdf7a` |
| tempo-control-and-warping | 9 Warping | snapshot was a redirect stub (4.7 KB); not yet read |

Version differences noticed against the current (Live 12) manual: Live 10 lists eight Follow
Actions (No Action, Stop, Play Again, Previous, Next, First, Last, Any, Other — "No Action" is
counted separately); "Jump" and scene Follow Actions are not part of Live 10.
