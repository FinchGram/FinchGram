#include "MainWindow.h"
#include "utils/SettingsManager.h"
#include <QApplication>

int main(int argc, char *argv[])
{
    QApplication app(argc, argv);
    // Set application properties for QSettings
    app.setOrganizationName("FinchGram");
    app.setApplicationName("FinchGram");

    // Initialize the SettingsManager singleton
    SettingsManager::instance();

    MainWindow w;
    w.show();
    
    // Store the return value from app.exec()
    int result = app.exec();
    
    // Destroy the SettingsManager singleton before returning
    SettingsManager::destroyInstance();
    
    return result;
}
