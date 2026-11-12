# Windows personal mod for 0.23.0

Rebuild the previous personal mod on the official v0.23.0 tag. Camera Raw retains Auto as a toggle, black-and-white treatment and automatic white balance; GenFill-K retains its separate plug-in window and layered variants. GenGate is intentionally uninstalled and its commands and preferences are unavailable.

The port keeps the current Camera Raw local-tone pipeline, smart-select refusal checks, document focus state and preference merge behavior. Tests use the current remove-stroke argument and verify the available preference sections. Eighteen legacy GenGate integration tests are explicitly ignored because that feature is unavailable; the Auto timing benchmark also remains ignored. Active-feature tests passed: 30 algorithm, 111 engine and 291 UI tests.

Credit: @kinower. This is a personal distribution, separate from the existing upstream Camera Raw PR #8392. No external image-generation request is made by the tests.
