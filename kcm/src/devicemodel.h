#pragma once

#include <QAbstractListModel>
#include <QDBusInterface>
#include <QList>
#include <QString>

struct PairedDevice {
    QString id;
    QString name;
    QString model;
    QString owner;
    QString securityLevel;
    QString verifiedBoot;
    quint64 pairedAt = 0;
    QString fingerprint;
};

/// Devices as the daemon reports them. The model owns no authoritative state: every change
/// goes to the daemon and the list is refreshed from it, so System Settings can never show
/// a device that is not actually paired.
class DeviceModel : public QAbstractListModel
{
    Q_OBJECT
    Q_PROPERTY(bool available READ available NOTIFY availableChanged)
    Q_PROPERTY(QString statusMessage READ statusMessage NOTIFY statusMessageChanged)
    Q_PROPERTY(QString pairingUri READ pairingUri NOTIFY pairingChanged)
    Q_PROPERTY(QString pairingCode READ pairingCode NOTIFY pairingChanged)
    Q_PROPERTY(bool pairing READ pairing NOTIFY pairingChanged)

public:
    enum Roles {
        IdRole = Qt::UserRole + 1,
        NameRole,
        ModelRole,
        OwnerRole,
        SecurityLevelRole,
        VerifiedBootRole,
        PairedAtRole,
        TrustedRole,
        FingerprintRole,
    };
    Q_ENUM(Roles)

    explicit DeviceModel(QObject *parent = nullptr);

    int rowCount(const QModelIndex &parent = QModelIndex()) const override;
    QVariant data(const QModelIndex &index, int role) const override;
    QHash<int, QByteArray> roleNames() const override;

    bool available() const;
    QString statusMessage() const;
    QString pairingUri() const;
    QString pairingCode() const;
    bool pairing() const;

    Q_INVOKABLE void refresh();
    Q_INVOKABLE void removeDevice(const QString &id);
    Q_INVOKABLE void beginPairing(const QString &label);
    Q_INVOKABLE void cancelPairing();
    Q_INVOKABLE int pairingSecondsLeft();

Q_SIGNALS:
    void availableChanged();
    void statusMessageChanged();
    void pairingChanged();

private:
    void setStatus(const QString &message);

    QDBusInterface m_manager;
    QList<PairedDevice> m_devices;
    QString m_status;
    QString m_pairingUri;
    QString m_pairingCode;
    /// Device count when the window opened. The daemon consumes the pairing token without
    /// announcing why, so a grown list is how success is told from a timeout.
    int m_pairingBaseline = 0;
};
