#include "SettingsManager.h"
#include <QMutexLocker>

// Initialize static members
SettingsManager* SettingsManager::settingsManagerInstance = nullptr;
QMutex SettingsManager::settingsManagerMutex;

SettingsManager::SettingsManager(QObject* parent)
    : QObject(parent)
    , qSettingsInstance(new QSettings(this))
{
    // QSettings will use the application organization and name set in main()
}

SettingsManager::~SettingsManager()
{
    if (qSettingsInstance) {
        qSettingsInstance->sync();
        delete qSettingsInstance;
        qSettingsInstance = nullptr;
    }
}

SettingsManager* SettingsManager::instance()
{
    QMutexLocker locker(&settingsManagerMutex);
    if (!settingsManagerInstance) {
        settingsManagerInstance = new SettingsManager(nullptr); // No parent as requested
    }
    return settingsManagerInstance;
}

void SettingsManager::destroyInstance()
{
    QMutexLocker locker(&settingsManagerMutex);
    if (settingsManagerInstance) {
        delete settingsManagerInstance;
        settingsManagerInstance = nullptr;
    }
}

void SettingsManager::setValue(const QString& key, const QVariant& value)
{
    if (qSettingsInstance) {
        qSettingsInstance->setValue(key, value);
    }
}

QVariant SettingsManager::value(const QString& key, const QVariant& defaultValue) const
{
    if (qSettingsInstance) {
        return qSettingsInstance->value(key, defaultValue);
    }
    return defaultValue;
}

bool SettingsManager::contains(const QString& key) const
{
    if (qSettingsInstance) {
        return qSettingsInstance->contains(key);
    }
    return false;
}

void SettingsManager::remove(const QString& key)
{
    if (qSettingsInstance) {
        qSettingsInstance->remove(key);
    }
}

void SettingsManager::clear()
{
    if (qSettingsInstance) {
        qSettingsInstance->clear();
    }
}

void SettingsManager::sync()
{
    if (qSettingsInstance) {
        qSettingsInstance->sync();
    }
} 