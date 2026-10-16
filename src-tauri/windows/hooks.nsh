!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegKey HKCU "Software\Google\Chrome\NativeMessagingHosts\com.novament.pulsedm"
  DeleteRegKey HKCU "Software\Microsoft\Edge\NativeMessagingHosts\com.novament.pulsedm"
  DeleteRegKey HKCU "Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\com.novament.pulsedm"
  DeleteRegKey HKCU "Software\Chromium\NativeMessagingHosts\com.novament.pulsedm"
  DeleteRegKey HKCU "Software\Mozilla\NativeMessagingHosts\com.novament.pulsedm"
!macroend
