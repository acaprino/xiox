# Third-party notices

PulisciPC studies and adapts parts of the projects below. Only MIT-licensed code is adapted; cleanup-cc has no licence and only informed our ideas (no code taken).

### kondo

Source: https://github.com/tbillington/kondo
Copyright (c) 2020 Trent Billington
Licensed under the MIT License. Portions of PulisciPC adapt ideas or code from this project; the full MIT text is reproduced at the end of this file.

### ai-session-tidy

Source: https://github.com/sooink/ai-session-tidy
Copyright (c) 2025 sooink
Licensed under the MIT License. Portions of PulisciPC adapt ideas or code from this project; the full MIT text is reproduced at the end of this file.

### claude-code-cleaner

Source: https://github.com/GarrickZ2/claude-code-cleaner
Copyright (c) 2026 GarrickZ2
Licensed under the MIT License. Portions of PulisciPC adapt ideas or code from this project; the full MIT text is reproduced at the end of this file.

### claude-chats-delete

Source: https://github.com/ataleckij/claude-chats-delete
Copyright (c) 2026 ataleckij
Licensed under the MIT License. Portions of PulisciPC adapt ideas or code from this project; the full MIT text is reproduced at the end of this file.

### mukade (vendored, compiled into the app)

Source: https://github.com/acaprino/mukade (same author as PulisciPC), commit dfde57e (2026-11-10).
Copyright (c) acaprino. MIT. Library-only copy in `vendor/mukade/` (see `vendor/mukade/VENDORED.md`); used to read the sessions of Claude Code, Codex, OpenCode and Pi (listings only, read-only).

### Winapp2 (rules downloaded at runtime)

Source: https://github.com/MoscaDotTo/Winapp2
Licensed under CC-BY-SA-4.0. PulisciPC downloads `Winapp2.ini` at runtime to read its cleaning rules: il file si scarica a runtime, non fa parte dell'eseguibile. The file is kept unmodified in the user's data folder and keeps its own licence.

### Embedded CleanerML cleaners

The eleven XML cleaners in `cleaners/` (Claude Code, Codex, Gemini CLI, OpenCode, Cargo, crash dumps, Go, JetBrains, npm, pip, GPU shaders) are compiled into the executable. They are authored or adapted for this app and are covered by the app's own licence.

### Also looked at (ideas only, no code)

- cleanup-cc: https://github.com/nicechencs/cleanup-cc (no licence)

## MIT License text

MIT License

Copyright (c) <year> <copyright holders listed above>

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.