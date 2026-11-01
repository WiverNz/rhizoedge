# Pico Technology vendor reference

This directory preserves the original vendor document used to implement and
validate the [PicoScope 2205A bench tooling](../../hardware/picoscope-2205a.md).
The PDF is stored verbatim, with all vendor copyright/license notices retained.
It has not been re-rendered, stripped, annotated, recompressed or edited.

| Field | Recorded value |
|---|---|
| Vendor | Pico Technology Ltd. |
| Title | PicoScope® 2000 Series PC Oscilloscopes — Programmer's Guide |
| Identifier/version | **ps2000pg.en-12**, confirmed from the downloaded cover/footer |
| File | [picoscope-2000-series-programmers-guide.pdf](picoscope-2000-series-programmers-guide.pdf) |
| Source | https://www.picotech.com/download/manuals/picoscope-2000-series-programmers-guide.pdf |
| Download date | **2026-11-01 (UTC)** |
| Size | **1,595,815 bytes**, 68 pages |
| SHA-256 | `450d6d0578e8137ed46554a4ce00577b822938625517380e5bee0a0cbf7d2278` |
| Hardware/API used here | **PicoScope 2205A / ps2000**; native Windows x64 backend controlled from WSL |
| Vendor copyright | Copyright © 2006–2022 Pico Technology Ltd. All rights reserved. |

## Notices and repository policy

Section 1.3, **Legal information** (printed page 6), states the SDK copying/
distribution condition: retain Pico Technology copyright statements. The full
conditions, including use with Pico products/data, remain in the unmodified PDF.
This vendor material retains its own notices; it is not relicensed as Rhizo
Edge source. Read the document's legal section when redistributing it.

Repository `.gitattributes` explicitly marks `*.pdf` as binary. No conflicting
policy against this vendor reference was found, so the requested original is
included rather than replaced by a link or an extracted text copy.

## Verify or reproduce the download

From the repository root:

```sh
sha256sum docs/vendor/pico/picoscope-2000-series-programmers-guide.pdf

# Download separately; never overwrite the archived copy without reviewing a change.
curl --fail --location --silent --show-error \
  'https://www.picotech.com/download/manuals/picoscope-2000-series-programmers-guide.pdf' \
  --output /tmp/picoscope-guide-candidate.pdf
printf '%s  %s\n' \
  '450d6d0578e8137ed46554a4ce00577b822938625517380e5bee0a0cbf7d2278' \
  '/tmp/picoscope-guide-candidate.pdf' | sha256sum --check -
```

If the vendor replaces the URL's contents, a mismatch is meaningful: inspect the
new document/version/legal notices before updating the archive and provenance.
Never remove copyright pages to force a size/hash match. Keep download date,
version, byte size and SHA-256 synchronized with any deliberate replacement.

The relevant API sections include `ps2000_get_timebase`, `ps2000_set_channel`,
`ps2000_set_trigger2`, `ps2000_run_block`, `ps2000_get_values`, and
`ps2000_set_sig_gen_built_in`. Device-specific limits and the measured probe/
bench response still govern what a capture can prove.
