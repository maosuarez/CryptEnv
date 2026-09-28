## MODIFIED Requirements

### Requirement: Cross-Platform Global Shortcut Registration and Lifecycle
The application MUST dynamically register and re-register the global shortcut to toggle application visibility according to user settings at runtime, without requiring an application restart. On application launch, the desktop backend MUST load the persisted hotkey from settings (falling back to a cross-platform default `Ctrl+Alt+Z` on Windows/Linux or `Cmd+Alt+Z` on macOS). When the user saves a new hotkey, the backend MUST unregister the previous shortcut and register the new shortcut immediately. A failure to register either the persisted hotkey or the default at startup MUST NOT prevent the application from starting: the application SHALL start without a global shortcut and SHALL tell the user in the GUI that the hotkey is unavailable and can be changed in Settings.

#### Scenario: Dynamic hotkey re-registration
- **WHEN** the user changes the global hotkey in Settings and saves settings
- **THEN** the backend unregisters the prior shortcut and registers the new shortcut with the operating system without restarting the application
- **AND** pressing the new shortcut immediately toggles the application window visibility.

#### Scenario: Startup hotkey initialization
- **WHEN** the application starts up
- **THEN** the backend reads the configured hotkey from vault settings and registers it as the global shortcut, or registers the platform default if unset.

#### Scenario: Hotkey owned by another application
- **WHEN** the application starts and both the persisted hotkey and the default are already registered by another application
- **THEN** the application starts normally without a global shortcut
- **AND** the GUI shows a notice that the hotkey is unavailable and can be changed in Settings.

#### Scenario: Cross-platform modifier adaptation
- **WHEN** the application is running on macOS
- **THEN** modifier keys adapt to macOS conventions (displaying `Cmd` instead of `Ctrl`), and shortcut parsing accepts macOS Command/Super modifiers.
