#pragma once
#include <QSettings>
#include <QObject>
#include <QMutex>

class SettingsManager : public QObject
{
    Q_OBJECT

public:
    // Get the singleton instance
    static SettingsManager* instance();
    
    // Destroy the singleton instance
    static void destroyInstance();
    
    // Settings access methods
    void setValue(const QString& key, const QVariant& value);
    QVariant value(const QString& key, const QVariant& defaultValue = QVariant()) const;
    bool contains(const QString& key) const;
    void remove(const QString& key);
    void clear();
    void sync();
    
    // Prevent copying
    SettingsManager(const SettingsManager&) = delete;
    SettingsManager& operator=(const SettingsManager&) = delete;
    
private:
    // Private constructor and destructor for singleton
    explicit SettingsManager(QObject* parent = nullptr);
    ~SettingsManager();
    
    static SettingsManager* settingsManagerInstance;
    static QMutex settingsManagerMutex;
    QSettings* qSettingsInstance;
}; 