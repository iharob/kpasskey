#include "devicemodel.h"

#include <KPluginFactory>
#include <KQuickConfigModule>

#include <qqml.h>

class KPasskeyKcm : public KQuickConfigModule
{
    Q_OBJECT
    Q_PROPERTY(DeviceModel *devices READ devices CONSTANT)

public:
    KPasskeyKcm(QObject *parent, const KPluginMetaData &data)
        : KQuickConfigModule(parent, data)
        , m_devices(new DeviceModel(this))
    {
        qmlRegisterAnonymousType<DeviceModel>("org.kde.kpasskey.kcm", 1);
    }

    DeviceModel *devices() const
    {
        return m_devices;
    }

private:
    DeviceModel *const m_devices;
};

K_PLUGIN_CLASS_WITH_JSON(KPasskeyKcm, "kcm_kpasskey.json")

#include "kpasskeykcm.moc"
