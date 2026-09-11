$ErrorActionPreference = 'Stop'
                $root = $env:RESTART_ENTRY_ROOT
                . (Join-Path $root 'codex-discord-rust-drain.ps1')
                $DrainPreparePath = Join-Path $root '.codex_discord_rust.drain.prepare'
                $DrainAckPath = Join-Path $root '.codex_discord_rust.drain.ack'
                $fence = Get-RestartDrainFence -Path (Join-Path $root '.codex_discord_rust.restart')
                function Get-VerifiedRuntimeProcess { $null }
                function Get-RuntimePid { 0 }
                try { Restore-RestartDrainAfterFailedLaunch -Fence $fence; throw 'changed fence accepted' }
                catch { if ($_.Exception.Message -notmatch 'state changed') { throw } }
                exit 0
