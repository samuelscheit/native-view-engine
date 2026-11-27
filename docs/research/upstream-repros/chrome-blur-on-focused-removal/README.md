# chrome-blur-on-focused-removal: already reported (no repro built)

**Verdict:** genuine Chromium spec violation, already reported and tracked. Nothing to file.

## Spec

HTML "removing steps": if the document's focused area is `removedNode`, it becomes the viewport. "This does not perform the unfocusing steps, focusing steps, or focus update steps, and thus no blur or change events are fired." Clarified by https://github.com/whatwg/html/pull/8392 (merged 2023-01-27). Gecko and WebKit follow it, and only Chromium fires `blur`/`focusout` (and `change`) synchronously on removal.

## Existing reports

- https://issues.chromium.org/issues/41484175, "blur events are fired while an element is being removed from the DOM, allowing it to be re-focused…". Open: the fix is behind a flag, and the tracker needs sign-in, so its exact state was not read. Fix CL: https://chromium-review.googlesource.com/8313948
- blink-dev "Intent to deprecate and remove: Stop sending blur events on element removal": https://groups.google.com/a/chromium.org/g/blink-dev/c/L1aI9JZTrBs. It stalled on web-compat concerns.
- Flag `OmitBlurEventOnElementRemoval` is still `status: "test"` on Chromium main (`third_party/blink/renderer/platform/runtime_enabled_features.json5`), so the spec behavior is not shipped by default.
- Related spec work: the `focusreset` event, https://github.com/whatwg/html/pull/13050, and https://github.com/WebKit/standards-positions/issues/732.
- MDN documents the difference: https://developer.mozilla.org/en-US/docs/Web/API/Element/blur_event

## Action

Do not file. A comment on 41484175 would add little: it would be a +1 with a use case (synchronous re-entrancy into a WASM host during DOM apply, i.e. a `RefCell` `BorrowMutError`). Starring the issue is enough. Our mitigation in docs/UPSTREAM.md stays correct: queue events raised during apply and flush them afterwards. It must keep working when Chromium ships the flag, because events then simply stop arriving.
