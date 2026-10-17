# Attribution

TexCraft's code and original assets (the app icon in `assets/icon/`, icons drawn in code,
templates, demo paper) are MIT OR Apache-2.0.

- **AI provider and model logos** (`assets/ai-logos/providers`, `assets/ai-logos/models`, shown
  in the AI Assistant and Settings → AI) are from [LobeHub Icons](https://github.com/lobehub/lobe-icons),
  Copyright (c) 2023 LobeHub, Apache-2.0 (licence text and NOTICE in `assets/ai-logos/`), copied
  unmodified by way of Argus. They are trademarks of their owners, used only to identify the
  services, and imply no endorsement or affiliation.
- **NUS crest** (`assets/nus/nus.png`, the SoCLaaS provider's logo) is the National University of
  Singapore's favicon, copied by way of Argus. It is NUS's mark and not openly licensed: the one
  exception to permissive-only assets, included at the maintainer's request only to identify the
  NUS School of Computing's SoCLaaS service. It implies no endorsement or affiliation.
- **AI Suggestions icon** (`assets/icons/copilot.svg`): drawn for TexCraft, at the maintainer's
  request, after the look of GitHub Copilot's mark (a pilot in goggles). GitHub Copilot is
  GitHub's trademark; the icon only marks the editor's AI suggestions and implies no endorsement
  or affiliation.
- **Windows installer.** `TexCraft-<version>-windows-x64-setup.exe` is built with
  [Inno Setup](https://jrsoftware.org/isinfo.php) (Copyright (C) 1997-2026 Jordan Russell and
  Martijn Laan, [licence](https://jrsoftware.org/files/is/license.txt)), whose setup and uninstall
  programs it contains. Free of charge for any use; the authors ask only commercial users to buy a
  licence, and TexCraft is not commercial.
- **No other third-party assets are bundled.** The UI and editor fonts (SF Pro, SF Mono, Hiragino Sans GB,
  Apple Symbols, STIX Two Math on macOS; Segoe UI/Consolas on Windows; DejaVu on Linux) are loaded
  from the operating system at run time when present, with egui's built-in fonts as fallback.
- **Clean-room.** The target app's look and behaviour were taken from public material only
  (see `docs/texifier-reference.md` §10). No code, icons, images, templates or text were copied.
- Rust dependencies keep their own licences (see `Cargo.lock`); notably hayro (PDF rendering,
  MIT/Apache-2.0), egui/eframe/egui_extras with resvg (MIT/Apache-2.0), muda (MIT/Apache-2.0)
  and aes-gcm (MIT/Apache-2.0, API keys at rest).
- Test fixtures under `crates/*/tests/data/` were produced locally with TeX Live 2022 from
  documents written for this project.
