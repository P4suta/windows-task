param(
    [ValidateSet('create', 'remove', 'probe')]
    [string]$Mode,
    [string]$Name
)

$ErrorActionPreference = 'Stop'
$accountStage = 'guard'
try {
    if ($Mode -ne 'probe' -and ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:WINDOWS_TASK_ACCOUNT_TESTS -ne '1')) {
        throw 'Account mutation requires explicitly acknowledged disposable CI'
    }
    if ($Mode -ne 'probe' -and $Name -notmatch '^wt[0-9a-f]{16}$') {
        throw 'Account name is outside the generated fixture namespace'
    }
    $accountStage = 'bindings'
        # Use the OS API: LocalAccounts cmdlets are absent on some hosted images.
        # This source contains no runtime password; only the stdin value enters it.
        Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class WindowsTaskFixtureAccounts {
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    public struct UserInfo {
        public string Name;
        public string Password;
        public uint PasswordAge;
        public uint Privilege;
        public string HomeDirectory;
        public string Comment;
        public uint Flags;
        public string ScriptPath;
    }
    [DllImport("netapi32.dll", CharSet = CharSet.Unicode)]
    private static extern uint NetUserAdd(string server, uint level, ref UserInfo user, out uint parameter);
    [DllImport("netapi32.dll", CharSet = CharSet.Unicode)]
    public static extern uint NetUserDel(string server, string user);
    public static uint Create(string name, string password) {
        var user = new UserInfo {
            Name = name, Password = password, Privilege = 1,
            // UF_SCRIPT | UF_NORMAL_ACCOUNT | UF_DONT_EXPIRE_PASSWD.
            Flags = 0x10201, Comment = "windows-task disposable acceptance account"
        };
        uint parameter;
        return NetUserAdd(null, 1, ref user, out parameter);
    }
}
'@
    $accountStage = $Mode
    switch ($Mode) {
        'create' {
            $accountPasswordText = [Console]::In.ReadLine()
            try {
                $accountStatus = [WindowsTaskFixtureAccounts]::Create($Name, $accountPasswordText)
                if ($accountStatus -ne 0) {
                    throw [ComponentModel.Win32Exception]::new([int]$accountStatus)
                }
            } finally {
                $accountPasswordText = $null
            }
        }
        'remove' {
            $accountStatus = [WindowsTaskFixtureAccounts]::NetUserDel($null, $Name)
            # NERR_UserNotFound also confirms cleanup after a failed create.
            if ($accountStatus -ne 0 -and $accountStatus -ne 2221) {
                throw [ComponentModel.Win32Exception]::new([int]$accountStatus)
            }
        }
        'probe' { throw 'SENTINEL_EXCEPTION_BODY' }
    }
} catch {
    @{
        hresult = $_.Exception.HResult
        category = [int]$_.CategoryInfo.Category
        error_type = $_.Exception.GetType().FullName
        phase = $accountStage
        native_code = if ($_.Exception -is [ComponentModel.Win32Exception]) { $_.Exception.NativeErrorCode } else { $null }
    } | ConvertTo-Json -Compress
    exit 1
}
