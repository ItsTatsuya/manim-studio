{ Microsoft-documented per-user/per-machine Evergreen registry detection. }
function HasEvergreenWebView2: Boolean;
var
  Version: String;
  Key: String;
begin
  Key := 'Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';
  Result := (RegQueryStringValue(HKCU, Key, 'pv', Version) and (Version <> '') and (Version <> '0.0.0.0')) or
    (RegQueryStringValue(HKLM32, Key, 'pv', Version) and (Version <> '') and (Version <> '0.0.0.0'));
end;

function EnsureEvergreenWebView2(const Installer: String): String;
var
  Code: Integer;
begin
  Result := '';
  if HasEvergreenWebView2 then exit;
  try
    ExtractTemporaryFile(Installer);
    if not Exec(ExpandConstant('{tmp}\') + Installer, '/silent /install', '', SW_HIDE, ewWaitUntilTerminated, Code) then
      RaiseException('Could not start the Microsoft WebView2 installer.');
    if ((Code <> 0) and (Code <> 3010)) or not HasEvergreenWebView2 then
      RaiseException('Microsoft WebView2 Runtime could not be prepared. Check your connection, or use the offline Studio installer.');
  except
    Result := GetExceptionMessage + #13#10 + 'Your existing app and saved work have not been changed.';
  end;
end;
