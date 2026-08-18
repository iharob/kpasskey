cmake_minimum_required(VERSION 3.25)
project(kcm_kpasskey VERSION 0.1.1 LANGUAGES CXX) # x-release-please-version

set(QT_MIN_VERSION 6.8.0)
set(KF_MIN_VERSION 6.0.0)

find_package(ECM ${KF_MIN_VERSION} REQUIRED NO_MODULE)
set(CMAKE_MODULE_PATH ${ECM_MODULE_PATH})

include(KDEInstallDirs)
include(KDECMakeSettings)
include(KDECompilerSettings NO_POLICY_SCOPE)
include(ECMQmlModule)

find_package(Qt6 ${QT_MIN_VERSION} REQUIRED COMPONENTS Core Gui Qml Quick DBus)
find_package(KF6 ${KF_MIN_VERSION} REQUIRED COMPONENTS KCMUtils CoreAddons I18n)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# A warning is a defect: fail the build rather than accumulate them.
add_compile_options(-Wall -Wextra -Werror -Wpedantic)

kcmutils_add_qml_kcm(kcm_kpasskey SOURCES
    src/kpasskeykcm.cpp
    src/devicemodel.cpp
)

target_link_libraries(kcm_kpasskey PRIVATE
    Qt6::Core
    Qt6::DBus
    Qt6::Qml
    KF6::KCMUtils
    KF6::CoreAddons
    KF6::I18n
)
