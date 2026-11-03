// X11 only. No root privileges or global input grabs; Deskflow owns capture.
#include <X11/Xlib.h>
#include <X11/Xatom.h>
#include <X11/keysym.h>
#include <X11/extensions/Xrandr.h>
#include <X11/extensions/XTest.h>
#include <limits.h>
#include <ctype.h>
#include <openssl/sha.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static void json_string(const char *s, int length) {
    putchar('"');
    for (int i = 0; i < length; ++i) {
        unsigned char c = (unsigned char)s[i];
        if (c == '"' || c == '\\') { putchar('\\'); putchar(c); }
        else if (c < 32 || c >= 127) printf("\\u%04x", c);
        else putchar(c);
    }
    putchar('"');
}
static void output_id(Display *d, Window root, XRRScreenResources *resources, RROutput output) {
    XRROutputInfo *info = XRRGetOutputInfo(d, resources, output);
    if (!info) { json_string("unknown", 7); return; }
    // EDID plus connector survives XID/CRTC changes after sleep and hotplug.
    char id[128];
    int length = snprintf(id, sizeof(id), "xrandr-");
    for (int i = 0; i < info->nameLen && i < 48; ++i) {
        unsigned char c = (unsigned char)info->name[i];
        id[length++] = (c < 128 && (isalnum(c) || c == '-' || c == '_')) ? (char)c : '_';
    }
    id[length++] = '-';
    Atom actual; int format; unsigned long count, after; unsigned char *bytes = NULL;
    Atom edid = XInternAtom(d, "EDID", True);
    unsigned char hash[SHA256_DIGEST_LENGTH];
    if (edid != None && XRRGetOutputProperty(d, output, edid, 0, 128, False, False,
        AnyPropertyType, &actual, &format, &count, &after, &bytes) == Success && format == 8 && bytes) {
        SHA256(bytes, count, hash);
    } else { SHA256((const unsigned char *)"", 0, hash); }
    for (int i = 0; i < 12; ++i) length += snprintf(id + length, sizeof(id) - (size_t)length, "%02x", hash[i]);
    if (bytes) XFree(bytes);
    json_string(id, length);
    XRRFreeOutputInfo(info);
    (void)root;
}
static int screens(Display *d) {
    Window root = DefaultRootWindow(d);
    int count = 0, first = 1;
    int major = 1, minor = 5;
    if (!XRRQueryVersion(d, &major, &minor) || major < 1 || (major == 1 && minor < 5)) {
        fputs("RandR 1.5 is required\n", stderr); return 3;
    }
    XRRScreenResources *resources = XRRGetScreenResourcesCurrent(d, root);
    if (!resources) return 3;
    XRRMonitorInfo *monitors = XRRGetMonitors(d, root, True, &count);
    putchar('[');
    for (int i = 0; i < count; ++i) {
        XRRMonitorInfo *m = &monitors[i];
        if (m->width <= 0 || m->height <= 0 || m->noutput < 1) continue;
        XRROutputInfo *info = XRRGetOutputInfo(d, resources, m->outputs[0]);
        if (!info) continue;
        if (!first) putchar(',');
        first = 0;
        fputs("{\"id\":", stdout); output_id(d, root, resources, m->outputs[0]);
        fputs(",\"name\":", stdout); json_string(info->name, info->nameLen);
        int internal = strncmp(info->name, "eDP", 3) == 0 || strncmp(info->name, "LVDS", 4) == 0;
        printf(",\"internal\":%s,\"bounds\":{\"x\":%d,\"y\":%d,\"width\":%d,\"height\":%d}}",
            internal ? "true" : "false", m->x, m->y, m->width, m->height);
        XRRFreeOutputInfo(info);
    }
    puts("]");
    if (monitors) XRRFreeMonitors(monitors);
    XRRFreeScreenResources(resources);
    return 0;
}
static int inject(Display *d, int host) {
    int event, error, major, minor;
    if (!XTestQueryExtension(d, &event, &error, &major, &minor)) return 3;
    KeyCode keys[] = { XKeysymToKeycode(d, XK_Control_L), XKeysymToKeycode(d, XK_Alt_L),
        XKeysymToKeycode(d, XK_Super_L), XKeysymToKeycode(d, XK_Shift_L), XKeysymToKeycode(d, host ? XK_F7 : XK_F8) };
    char state[32]; XQueryKeymap(d, state);
    int pressed[5] = {0};
    for (int i = 0; i < 5; ++i) if (!keys[i]) return 3;
    for (int i = 0; i < 5; ++i) {
        // Do not release modifiers already held by the physical user.
        if (!(state[keys[i] / 8] & (1 << (keys[i] % 8)))) {
            XTestFakeKeyEvent(d, keys[i], True, 0); pressed[i] = 1;
        }
    }
    XSync(d, False);
    for (int i = 4; i >= 0; --i) if (pressed[i]) XTestFakeKeyEvent(d, keys[i], False, 0);
    XSync(d, False);
    puts("sent"); return 0;
}
int main(int argc, char **argv) {
    const char *action = argc > 1 ? argv[1] : "check";
    const char *session = getenv("XDG_SESSION_TYPE");
    if (getenv("WAYLAND_DISPLAY") || (session && strcmp(session, "wayland") == 0)) {
        fputs("Wayland is not supported for input control; select Ubuntu on Xorg\n", stderr); return 3;
    }
    Display *d = XOpenDisplay(NULL);
    if (!d) { fputs("Cannot open X11 display\n", stderr); return 3; }
    int result = 0;
    if (strcmp(action, "screens") == 0) result = screens(d);
    else if (strcmp(action, "check") == 0) {
        int a,b,c,e;
        if (!XTestQueryExtension(d, &a, &b, &c, &e)) result = 3;
        else if (!XKeysymToKeycode(d, XK_Super_L) || !XKeysymToKeycode(d, XK_F7) || !XKeysymToKeycode(d, XK_F8)) result = 3;
        else puts("true");
    } else if (strcmp(action, "host") == 0 || strcmp(action, "peer") == 0) result = inject(d, strcmp(action, "host") == 0);
    else if (strcmp(action, "pointer") == 0 && argc == 4) {
        char *endx, *endy;
        double x = strtod(argv[2], &endx), y = strtod(argv[3], &endy);
        if (*endx || *endy || !isfinite(x) || !isfinite(y) || x < INT_MIN || x > INT_MAX || y < INT_MIN || y > INT_MAX) result = 2;
        else { XWarpPointer(d, None, DefaultRootWindow(d), 0, 0, 0, 0, (int)x, (int)y); XSync(d, False); puts("positioned"); }
    } else result = 2;
    XCloseDisplay(d); return result;
}
