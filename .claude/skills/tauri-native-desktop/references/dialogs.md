# External windows and dialogs: relationship, modality, close

Part of the tauri-native-desktop references: versions, scope and limits are in [guide.md](guide.md). Bracketed names are sources; their URLs are in [sources.md](sources.md).

Observed Rust API:

```rust
pub fn parent(self, parent: &WebviewWindow<R>) -> Result<Self>
```

| Platform | Meaning of `parent` | Modality |
|---|---|---|
| Windows | Window owner, with z-order and lifecycle relationship | Input blocking is separate |
| Linux | Only `transient_for` | GTK `set_modal(true)` is a separate operation |
| macOS | AppKit child window | Not equivalent to a document-modal sheet |

On Windows `owner(&WebviewWindow)` is explicit; `parent_raw(HWND)` instead creates a `WS_CHILD` window confined to the client area: it is not the choice for an external dialog. The default docs.rs page uses the Linux target and does not show `owner`, `owner_raw` and `parent_raw`: read the Windows target page. Do not invent a `.modal(true)` builder for Tauri 2.12.5: there is no `modal` config key either, and the open issues found (for example [tauri#3326]) concern full-screen transparent overlays, not a generic modal API. [Windows builder], [tauri-utils 2.10.2 config.rs], [tao Linux window], [GTK modal], [AppKit child], [AppKit sheet].

```rust
// Illustrative, compile against your lockfile: Linux modality on top of `parent`.
#[cfg(target_os = "linux")]
{ use gtk::prelude::GtkWindowExt; dialog.gtk_window()?.set_modal(true); }
```

For a modal Windows form, `set_enabled(false)` on the main window is a possible part of the adapter; Win32 requires re-enabling the owner before destroying the dialog to keep activation correct. It is a strategy to test, not a portable, already proven recipe. [Tauri WebviewWindow], [EnableWindow].

## Proposed manager contract

The coordinator owns an identified session and a single terminal completion. The following phases describe responsibilities, not a ready-to-copy snippet:

1. Reactivate the existing session or create one with an allowed type/label/route. Create the window in the async context/thread allowed by the version: Tauri documents a Windows deadlock risk in synchronous commands/event handlers.
2. Prepare content, initial data and a readiness handshake. Size to the monitor's work area; keep actions and content reachable with large text. Show and focus the dialog first; disable the main window only once a path that re-enables it exists.
3. If required, apply modality with the platform adapter. Ownership, `always_on_top`, an HTML overlay and `inert` in the main page do not by themselves prove native input blocking.
4. Handle confirm/cancel/X/Escape according to the contract; on Windows release the block before the controlled destruction. Cover load failure, API errors, forced close, owner destruction and shutdown. Make cleanup idempotent and for nested dialogs define the release order, or count the blocks so the main window is re-enabled only when the last one ends.
5. Deliver the result exactly once, remove listeners/sessions and restore focus if the original control still exists. If confirmation fails, keep the draft and a readable error instead of closing automatically.

The documented APIs set the constraints; this orchestration is an engineering proposal to validate in the runtime. On macOS a sheet suits a brief, scoped task tied to its context, and only one sheet is shown at a time; for long or complex flows Apple suggests a new window or full screen; for repeated input whose effect the user watches (find and replace) a panel. "Document-modal" is AppKit API terminology, not HIG wording. [Apple Sheets], [Modality], [beginSheet].

In Windows handling do not rely only on `Destroyed` for the release: it is already too late for the documented activation order. Intercept `CloseRequested` and converge on the finalizer without `close()` recursion (deduced from the documented `close()` contract); `destroy()` can skip the normal path. Restore only the block owned by the session, respecting any other reasons for disabling. `tauri-runtime-wry` 2.12.3 implements `set_enabled` per platform: Windows calls `EnableWindow` directly, Linux `gtk_window().set_sensitive`, macOS attaches a dummy NSWindow as a sheet, which requires the main thread. It queues requests coming from other threads: a successful `Result<()>` does not prove the OS effect has already happened. Establish the order in the coordinator and test it, without blocking the UI thread while waiting for the form. [WebviewWindow source], [runtime lib.rs], [runtime macOS window], [runtime Windows window], [EnableWindow].

## Full close, not just unmounting the form

For React frontends also cover StrictMode: in development the effects' setup/cleanup cycle is repeated. Do not create and immediately destroy OS windows in that cycle without coordination. A deferred, cancellable open before creating the resource is one possible solution; a test must show a single actual open and the runtime must confirm repeated closing and reopening. Do not disable StrictMode to hide the defect.

For every converted surface define an event → result → cleanup → focus table. Cover successful confirm, rejected confirm, Cancel, Escape, X/Alt+F4, owner close, unexpected destruction and app exit. Confirmation completes only after the authoritative operation succeeds; an error keeps the draft and the message in the form. Unmounting the form's component does not prove the OS host was closed: verify that the window really disappears and the main window becomes usable again, with no empty hosts, blocks or leftover listeners. Late events and repeated closes must converge on the same idempotent completion.

Choose one close decider per window, Rust or JS. The mere presence of a JS `onCloseRequested` listener makes the Rust side always call `prevent_close()` and moves the decision to the frontend, even if a Rust handler exists; the JS wrapper awaits the handler and then calls `destroy()`. [manager/window.rs], [window.ts].

Verify the concrete close path in the shipped runtime. The wry source for WebView2 (0.57.1, unchanged in 0.57.2) registers `WindowCloseRequested` by directly calling `DestroyWindow(hwnd)`: JavaScript `window.close()` bypasses `CloseRequested`. From the frontend close with `getCurrentWindow().close()`, never with `window.close()`. In an adapter based on a same-origin portal (see "Alternative for trusted forms" in [boundaries.md](boundaries.md)), route the unmount to the shell's coordinator and test the real close; do not destroy a controller inside its own COM callback. [wry 0.57.2 source].
