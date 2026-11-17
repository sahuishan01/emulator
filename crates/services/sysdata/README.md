# Original system firmware content

This project ships its own replacement for console system data. Nothing here
comes from Nintendo firmware, and no user firmware dump is required.

- `fonts/DejaVuSans.ttf`: DejaVu Sans (free license, see
  `fonts/DejaVuSans.LICENSE`). `pl:u` serves it for every shared font slot as
  a plain sfnt after the 8-byte header, the way the console exposes decoded
  system fonts.
  CJK and console-specific symbol glyphs are not covered yet.
- Time zones: `ITimeZoneService` uses its own fixed-offset zone table
  (`UTC`, `GMT`, `Etc/UTC`, `Etc/GMT`, `Etc/GMT±N`) and its own rule format; no
  tzdata archive. Other location names fail with TimeZoneNotFound.
