#include "devicemodel.h"

#include <KLocalizedString>

#include <QDBusArgument>
#include <QDBusConnection>
#include <QDBusInterface>
#include <QDBusObjectPath>
#include <QDBusReply>
#include <QDateTime>
#include <QVariantMap>

namespace
{
// The spike daemon runs on the session bus; the shipping daemon owns the same interface on
// the system bus. Only this constant changes.
constexpr QLatin1StringView ServiceName{"org.kpasskey"};
constexpr QLatin1StringView ObjectPath{"/org/kpasskey/Manager1"};
constexpr QLatin1StringView InterfaceName{"org.kpasskey.Manager1"};

constexpr QLatin1StringView SystemdService{"org.freedesktop.systemd1"};
constexpr QLatin1StringView SystemdPath{"/org/freedesktop/systemd1"};
constexpr QLatin1StringView SystemdManager{"org.freedesktop.systemd1.Manager"};
constexpr QLatin1StringView SystemdUnit{"org.freedesktop.systemd1.Unit"};
constexpr QLatin1StringView PropInterface{"org.freedesktop.DBus.Properties"};

/// Returns the ActiveState of a systemd unit, or an empty string when it cannot be queried.
QString activeState(const QDBusConnection &bus, const QString &unitName)
{
    QDBusInterface manager{SystemdService, SystemdPath, SystemdManager, bus};
    const QDBusReply<QDBusObjectPath> unit = manager.call(QStringLiteral("GetUnit"), unitName);
    if (!unit.isValid()) {
        return {};
    }

    QDBusInterface properties{SystemdService, unit.value().path(), PropInterface, bus};
    const QDBusReply<QVariant> state =
        properties.call(QStringLiteral("Get"), SystemdUnit, QStringLiteral("ActiveState"));
    if (!state.isValid()) {
        return {};
    }
    return state.value().toString();
}
}

DeviceModel::DeviceModel(QObject *parent)
    : QAbstractListModel(parent)
    , m_manager(ServiceName, ObjectPath, InterfaceName, QDBusConnection::sessionBus())
{
    QDBusConnection::sessionBus().connect(ServiceName,
                                          ObjectPath,
                                          InterfaceName,
                                          QStringLiteral("DevicesChanged"),
                                          this,
                                          SLOT(refresh()));
    refresh();
}

int DeviceModel::rowCount(const QModelIndex &parent) const
{
    return parent.isValid() ? 0 : static_cast<int>(m_devices.size());
}

QVariant DeviceModel::data(const QModelIndex &index, int role) const
{
    if (!index.isValid() || index.row() < 0 || index.row() >= m_devices.size()) {
        return {};
    }

    const PairedDevice &device = m_devices.at(index.row());
    switch (role) {
    case IdRole:
        return device.id;
    case NameRole:
        return device.name;
    case ModelRole:
        return device.model;
    case OwnerRole:
        return device.owner;
    case SecurityLevelRole:
        return device.securityLevel;
    case VerifiedBootRole:
        return device.verifiedBoot;
    case PairedAtRole:
        return QDateTime::fromSecsSinceEpoch(static_cast<qint64>(device.pairedAt))
            .toString(Qt::TextDate);
    case TrustedRole:
        // Software-backed keys are refused at pairing; anything else is hardware-backed.
        return device.securityLevel == QLatin1String("StrongBox")
            || device.securityLevel == QLatin1String("TrustedEnvironment");
    case FingerprintRole:
        return device.fingerprint;
    default:
        return {};
    }
}

QHash<int, QByteArray> DeviceModel::roleNames() const
{
    return {
        {IdRole, "deviceId"},
        {NameRole, "name"},
        {ModelRole, "model"},
        {OwnerRole, "owner"},
        {SecurityLevelRole, "securityLevel"},
        {VerifiedBootRole, "verifiedBoot"},
        {PairedAtRole, "pairedAt"},
        {TrustedRole, "hardwareBacked"},
        {FingerprintRole, "fingerprint"},
    };
}

QVariantMap DeviceModel::deviceDetails(int row) const
{
    if (row < 0 || row >= static_cast<int>(m_devices.size())) {
        return {};
    }

    const PairedDevice &device = m_devices.at(row);
    const bool hardwareBacked = device.securityLevel == QLatin1String("StrongBox")
        || device.securityLevel == QLatin1String("TrustedEnvironment");

    return {
        {QStringLiteral("deviceId"), device.id},
        {QStringLiteral("name"), device.name},
        {QStringLiteral("model"), device.model},
        {QStringLiteral("owner"), device.owner},
        {QStringLiteral("securityLevel"), device.securityLevel},
        {QStringLiteral("verifiedBoot"), device.verifiedBoot},
        {QStringLiteral("pairedAt"),
         QDateTime::fromSecsSinceEpoch(static_cast<qint64>(device.pairedAt)).toString(Qt::TextDate)},
        {QStringLiteral("hardwareBacked"), hardwareBacked},
        {QStringLiteral("fingerprint"), device.fingerprint},
    };
}

bool DeviceModel::available() const
{
    return m_manager.isValid();
}

QString DeviceModel::statusMessage() const
{
    return m_status;
}

QString DeviceModel::serviceStatus() const
{
    return m_serviceStatus;
}

void DeviceModel::setServiceStatus(const QString &message)
{
    if (m_serviceStatus == message) {
        return;
    }
    m_serviceStatus = message;
    Q_EMIT serviceStatusChanged();
}

QString DeviceModel::pairingUri() const
{
    return m_pairingUri;
}

QString DeviceModel::pairingCode() const
{
    return m_pairingCode;
}

bool DeviceModel::pairing() const
{
    return !m_pairingUri.isEmpty();
}

void DeviceModel::setStatus(const QString &message)
{
    if (m_status == message) {
        return;
    }
    m_status = message;
    Q_EMIT statusMessageChanged();
}

void DeviceModel::refresh()
{
    if (!m_manager.isValid()) {
        setStatus(i18n("The kpasskey service is not running."));
        setServiceStatus(i18n("The kpasskey D-Bus service is not running."));
        Q_EMIT availableChanged();
        return;
    }

    const QDBusMessage reply = m_manager.call(QStringLiteral("ListDevices"));
    if (reply.type() != QDBusMessage::ReplyMessage || reply.arguments().isEmpty()) {
        setStatus(i18n("Could not read the device list: %1", reply.errorMessage()));
        checkServices();
        return;
    }

    QList<PairedDevice> devices;
    const QDBusArgument argument = reply.arguments().first().value<QDBusArgument>();
    argument.beginArray();
    while (!argument.atEnd()) {
        PairedDevice device;
        argument.beginStructure();
        argument >> device.id >> device.name >> device.model >> device.owner
            >> device.securityLevel >> device.verifiedBoot >> device.pairedAt
            >> device.fingerprint;
        argument.endStructure();
        devices.append(device);
    }
    argument.endArray();

    beginResetModel();
    m_devices = devices;
    endResetModel();

    setStatus(m_devices.isEmpty() ? i18n("No phone is paired yet.") : QString());
    checkServices();
    Q_EMIT availableChanged();
}

void DeviceModel::checkServices()
{
    QStringList problems;

    const QString daemonState = activeState(QDBusConnection::sessionBus(), QStringLiteral("kpasskey.service"));
    if (daemonState != QLatin1String("active")) {
        problems.append(i18n("kpasskey.service is %1", daemonState.isEmpty() ? i18n("not loaded") : daemonState));
    }

    const QString fidoState = activeState(QDBusConnection::systemBus(), QStringLiteral("kpasskey-fido.service"));
    if (fidoState != QLatin1String("active")) {
        problems.append(i18n("kpasskey-fido.service is %1", fidoState.isEmpty() ? i18n("not loaded") : fidoState));
    }

    if (problems.isEmpty()) {
        setServiceStatus(QString());
    } else {
        setServiceStatus(i18n("Some services are not running: %1", problems.join(QLatin1String(", "))));
    }
}

void DeviceModel::removeDevice(const QString &id)
{
    const QDBusReply<bool> reply = m_manager.call(QStringLiteral("RemoveDevice"), id);
    if (!reply.isValid()) {
        setStatus(i18n("Could not remove the phone: %1", reply.error().message()));
        return;
    }
    if (!reply.value()) {
        setStatus(i18n("That phone was already removed."));
    }
    refresh();
}

void DeviceModel::beginPairing(const QString &label)
{
    const QDBusMessage reply = m_manager.call(QStringLiteral("BeginPairing"), label);
    if (reply.type() != QDBusMessage::ReplyMessage || reply.arguments().size() < 2) {
        setStatus(i18n("Could not start pairing: %1", reply.errorMessage()));
        return;
    }
    m_pairingUri = reply.arguments().at(0).toString();
    m_pairingCode = reply.arguments().at(1).toString();
    m_pairingBaseline = static_cast<int>(m_devices.size());
    setStatus(QString());
    Q_EMIT pairingChanged();
}

void DeviceModel::cancelPairing()
{
    m_manager.call(QStringLiteral("CancelPairing"));
    m_pairingUri.clear();
    m_pairingCode.clear();
    Q_EMIT pairingChanged();
    refresh();
}

int DeviceModel::pairingSecondsLeft()
{
    const QDBusReply<uint> reply = m_manager.call(QStringLiteral("PairingSecondsLeft"));
    if (!reply.isValid()) {
        return 0;
    }
    const int seconds = static_cast<int>(reply.value());
    if (seconds == 0 && pairing()) {
        m_pairingUri.clear();
        m_pairingCode.clear();
        Q_EMIT pairingChanged();
        refresh();
        // refresh() clears the status when devices exist, so say what happened after it.
        setStatus(static_cast<int>(m_devices.size()) > m_pairingBaseline
                      ? i18n("Phone paired.")
                      : i18n("Pairing timed out. Try again."));
    }
    return seconds;
}

#include "moc_devicemodel.cpp"
