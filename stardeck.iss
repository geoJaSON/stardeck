[Setup]
AppName=Stardeck
AppVersion=1.0.0
DefaultDirName={autopf}\Stardeck
DefaultGroupName=Stardeck
UninstallDisplayIcon={app}\stardeck.exe
Compression=lzma2
SolidCompression=yes
OutputDir=Output
OutputBaseFilename=stardeck_1.0.0_setup
SetupIconFile=icon.ico
PrivilegesRequired=lowest

[Files]
Source: "target\release\stardeck.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Stardeck"; Filename: "{app}\stardeck.exe"
Name: "{autodesktop}\Stardeck"; Filename: "{app}\stardeck.exe"

[Run]
Filename: "{app}\stardeck.exe"; Description: "Launch Stardeck"; Flags: postinstall nowait skipifsilent
