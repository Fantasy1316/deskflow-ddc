// SPDX-License-Identifier: MIT
// Pure geometry shared by the pinned engine patch and its device-free tests.
#pragma once
#include <algorithm>
#include <cmath>

namespace deskflow_ddc {
struct Rect { double x, y, width, height; };
inline bool valid(const Rect &r) {
    return std::isfinite(r.x) && std::isfinite(r.y) && std::isfinite(r.width)
        && std::isfinite(r.height) && r.width > 2 && r.height > 2;
}
inline bool contains(const Rect &r, double x, double y) {
    return x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height;
}
// side is the primary's side relative to the peer: left/right/top/bottom.
inline bool primaryEntry(const Rect &all, const Rect &visible, const char side,
                         int &x, int &y) {
    if (!valid(all) || !valid(visible) || contains(visible, x, y)
        || visible.x < all.x || visible.y < all.y
        || visible.x + visible.width > all.x + all.width
        || visible.y + visible.height > all.y + all.height) return false;
    const double inset = 2;
    const bool horizontal = side == 'l' || side == 'r';
    const double fraction = horizontal
        ? (y - all.y) / (all.height - 1) : (x - all.x) / (all.width - 1);
    double px = visible.x + std::clamp(fraction, 0.0, 1.0) * (visible.width - 1);
    double py = visible.y + std::clamp(fraction, 0.0, 1.0) * (visible.height - 1);
    switch (side) {
    case 'l': px = visible.x + visible.width - inset - std::max(0.0, all.x + all.width - 1 - x); break;
    case 'r': px = visible.x + inset + std::max(0.0, x - all.x); break;
    case 't': py = visible.y + visible.height - inset - std::max(0.0, all.y + all.height - 1 - y); break;
    case 'b': py = visible.y + inset + std::max(0.0, y - all.y); break;
    default: return false;
    }
    x = static_cast<int>(std::lround(std::clamp(px, visible.x + inset, visible.x + visible.width - inset)));
    y = static_cast<int>(std::lround(std::clamp(py, visible.y + inset, visible.y + visible.height - inset)));
    return true;
}
}
