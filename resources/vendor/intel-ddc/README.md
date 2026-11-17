# Intel DDC implementation reference

The original MIT helper in `native/mac-intel-ddc.m` and `native/mac-intel-ddc.h`
uses macOS IOKit I2C interfaces. The transport and framebuffer discovery were
informed by MonitorControl's `MonitorControl/Support/IntelDDC.swift`:

- Repository: https://github.com/MonitorControl/MonitorControl
- Reference revision: `7cfe64500d19cbbcbf82121a91c826f658a403bd`
- Authors credited by that file: MonitorControl, @JoniVR, @theOneyouseek,
  @waydabber and others; adapted from IntelDDC.swift by @reitermarkus.
- Upstream MIT license: `MONITORCONTROL-LICENSE`, included here.

Local code adds UUID selection, ambiguous-match rejection, independent display
enumeration, strict DDC reply validation, fixed-length packets, input-alt
addressing, bounded read attempts and explicit errors. It does not depend on
MonitorControl being installed and does not use a software brightness overlay.
