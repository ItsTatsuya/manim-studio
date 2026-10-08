{ Reserve 16 characters for Inno replacement names below the legacy path limit.
  Reject unsupported destinations before InstallDelete or file replacement runs. }
function ValidateInstallerPath(RelativeLength: Integer): String;
begin
  Result := '';
  if Length(AddBackslash(ExpandConstant('{app}'))) + RelativeLength > 243 then
    Result := 'The installation folder is too long for the bundled rendering tools.' + #13#10 +
      'Choose a shorter folder, such as the default location, then retry setup.' + #13#10 +
      'Your existing app and saved work have not been changed.';
end;

function PreparedPathLength(const Root, Relative: String): Integer;
var
  Entry: TFindRec;
  Name: String;
  Count: Integer;
begin
  Result := Length(Relative);
  if FindFirst(AddBackslash(Root) + Relative + '*', Entry) then begin
    try
      repeat
        if (Entry.Name <> '.') and (Entry.Name <> '..') then begin
          Name := Relative + Entry.Name;
          Count := Length(Name);
          if (Entry.Attributes and FILE_ATTRIBUTE_DIRECTORY) <> 0 then
            Count := PreparedPathLength(Root, Name + '\');
          if Count > Result then Result := Count;
        end;
      until not FindNext(Entry);
    finally
      FindClose(Entry);
    end;
  end else
    RaiseException('Prepared rendering tools are missing or unreadable. Restart setup to prepare them again.');
end;

function ValidatePreparedPaths: String;
var
  Root: String;
  Count: Integer;
begin
  try
    Root := ExpandConstant('{tmp}\prepared');
    if not DirExists(Root) then
      RaiseException('Prepared rendering tools are missing. Restart setup to prepare them again.');
    Count := PreparedPathLength(Root, '');
    if Count = 0 then
      RaiseException('Prepared rendering tools are empty. Restart setup to prepare them again.');
    Result := ValidateInstallerPath(Count);
  except
    Result := GetExceptionMessage + #13#10 +
      'Your existing app and saved work have not been changed.';
  end;
end;
