// SPDX-License-Identifier: MIT
// Local-only configuration. Never part of the Deskflow wire protocol.
#pragma once
#ifdef __APPLE__
#include "DdcPrimaryEntry.h"
#include <ApplicationServices/ApplicationServices.h>
#include <QFile>
#include <QJsonDocument>
#include <QJsonObject>

inline bool ddcMapPrimaryEntry(const std::string &peer, int ax, int ay, int aw, int ah, int &x, int &y,
                               bool recovery = false) {
    const auto path = qgetenv("DESKFLOW_DDC_PRIMARY_ROUTE");
    if (path.isEmpty()) return false;
    QFile file(QString::fromUtf8(path));
    if (!file.open(QIODevice::ReadOnly) || file.size() > 2048) return false;
    const auto object = QJsonDocument::fromJson(file.readAll()).object();
    if (object.value("schema").toInt() != 1 || !object.value("enabled").toBool()
        || object.value("peer").toString().toStdString() != peer) return false;
    bool ok = false;
    const auto display = object.value("displayId").toString().toUInt(&ok);
    if (!ok || !CGDisplayIsActive(display)) return false;
    const auto rect = CGDisplayBounds(display);
    const auto saved = object.value("bounds").toObject();
    if (rect.origin.x != saved.value("x").toDouble() || rect.origin.y != saved.value("y").toDouble()
        || rect.size.width != saved.value("width").toDouble() || rect.size.height != saved.value("height").toDouble()) return false;
    const deskflow_ddc::Rect visible{rect.origin.x, rect.origin.y, rect.size.width, rect.size.height};
    if (recovery) {
        x = static_cast<int>(visible.x + visible.width / 2);
        y = static_cast<int>(visible.y + visible.height / 2);
        return true;
    }
    const auto side = object.value("side").toString();
    if (side != "left" && side != "right" && side != "top" && side != "bottom") return false;
    return deskflow_ddc::primaryEntry({double(ax), double(ay), double(aw), double(ah)}, visible,
                                     side.at(0).toLatin1(), x, y);
}
#else
inline bool ddcMapPrimaryEntry(const std::string &, int, int, int, int, int &, int &, bool = false) { return false; }
#endif
