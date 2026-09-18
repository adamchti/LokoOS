<#
.SYNOPSIS
    Prints the ELF header and program headers of a LokoOS kernel image.

.DESCRIPTION
    A deliberately dependency-free ELF64 reader. The LLVM binutils that ship
    with some Rust toolchains are not present on every developer machine, and
    the handful of facts that matter for a kernel image -- is it really a
    64-bit x86 executable, where does it start, and are its segments mapped
    with sane permissions -- are a few dozen bytes into the file.

    Checking segment flags matters: a kernel whose text segment is writable, or
    whose data segment is executable, has given away W^X before userland has
    even started.

.EXAMPLE
    ./tools/elfinfo.ps1 target/x86_64-unknown-none/release/loko-kernel
#>
param(
    [Parameter(Mandatory = $true)]
    [string]$Path
)

$ErrorActionPreference = 'Stop'

if (-not (Test-Path $Path)) { throw "No such file: $Path" }
$bytes = [System.IO.File]::ReadAllBytes((Resolve-Path $Path))
if ($bytes.Length -lt 64) { throw "Too small to be an ELF file" }

if ($bytes[0] -ne 0x7F -or $bytes[1] -ne 0x45 -or $bytes[2] -ne 0x4C -or $bytes[3] -ne 0x46) {
    throw "Not an ELF file (bad magic)"
}

$class = if ($bytes[4] -eq 2) { 'ELF64' } else { 'ELF32' }
$endian = if ($bytes[5] -eq 1) { 'little-endian' } else { 'big-endian' }
$eType = [BitConverter]::ToUInt16($bytes, 0x10)
$eMachine = [BitConverter]::ToUInt16($bytes, 0x12)
$eEntry = [BitConverter]::ToUInt64($bytes, 0x18)
$ePhoff = [BitConverter]::ToUInt64($bytes, 0x20)
$ePhentsize = [BitConverter]::ToUInt16($bytes, 0x36)
$ePhnum = [BitConverter]::ToUInt16($bytes, 0x38)

$typeName = switch ($eType) { 1 { 'REL' } 2 { 'EXEC' } 3 { 'DYN' } 4 { 'CORE' } default { "unknown ($eType)" } }
$machineName = if ($eMachine -eq 0x3E) { 'x86-64' } else { "machine 0x{0:X}" -f $eMachine }

"File     : $Path"
"Size     : {0:N0} bytes" -f $bytes.Length
"Class    : $class, $endian"
"Type     : $typeName"
"Machine  : $machineName"
"Entry    : 0x{0:X16}" -f $eEntry
"Segments : $ePhnum"
""
"{0,-8} {1,-20} {2,-12} {3,-12} {4}" -f 'Type', 'VirtAddr', 'FileSize', 'MemSize', 'Flags'
"{0,-8} {1,-20} {2,-12} {3,-12} {4}" -f '----', '--------', '--------', '-------', '-----'

$problems = @()
for ($i = 0; $i -lt $ePhnum; $i++) {
    $off = [int]$ePhoff + ($i * $ePhentsize)
    $pType = [BitConverter]::ToUInt32($bytes, $off)
    $pFlags = [BitConverter]::ToUInt32($bytes, $off + 4)
    $pVaddr = [BitConverter]::ToUInt64($bytes, $off + 0x10)
    $pFilesz = [BitConverter]::ToUInt64($bytes, $off + 0x20)
    $pMemsz = [BitConverter]::ToUInt64($bytes, $off + 0x28)

    $ptName = switch ($pType) { 1 { 'LOAD' } 2 { 'DYNAMIC' } 4 { 'NOTE' } 6 { 'PHDR' } 0x6474e551 { 'GNU_STACK' } default { "0x{0:X}" -f $pType } }
    $r = if ($pFlags -band 4) { 'R' } else { '-' }
    $w = if ($pFlags -band 2) { 'W' } else { '-' }
    $x = if ($pFlags -band 1) { 'X' } else { '-' }
    $flagStr = "$r$w$x"

    "{0,-8} 0x{1:X16}   {2,-12:N0} {3,-12:N0} {4}" -f $ptName, $pVaddr, $pFilesz, $pMemsz, $flagStr

    if ($pType -eq 1 -and ($pFlags -band 2) -and ($pFlags -band 1)) {
        $problems += ("segment at 0x{0:X} is both writable and executable" -f $pVaddr)
    }
}

""
if ($problems.Count -gt 0) {
    foreach ($p in $problems) { "W^X VIOLATION: $p" }
    exit 1
}
"W^X: no segment is both writable and executable."
