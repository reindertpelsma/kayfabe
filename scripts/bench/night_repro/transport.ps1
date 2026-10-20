# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Export explicit JSON values, never extended PowerShell provider objects.
function ConvertTo-N1TransportValue($Value) {
    if($null -eq $Value) { return $null }
    if($Value -is [string]) { return ('{0}' -f $Value) }
    if($Value -is [bool]) { return [bool]$Value }
    if($Value -is [datetime]) { return $Value.ToString('o',[Globalization.CultureInfo]::InvariantCulture) }
    if($Value -is [ValueType]) { return $Value.PSObject.BaseObject }
    if($Value -is [Collections.IDictionary]) {
        $out=@{}
        foreach($key in $Value.Keys) { $out[('{0}' -f $key)]=ConvertTo-N1TransportValue $Value[$key] }
        return $out
    }
    if($Value -is [Collections.IEnumerable]) {
        $items=@(foreach($item in $Value) { ConvertTo-N1TransportValue $item })
        return ,$items
    }
    $out=@{}
    foreach($property in $Value.PSObject.Properties) {
        if($property.Name -notin @('PSPath','PSParentPath','PSChildName','PSDrive','PSProvider','ReadCount')) {
            $out[$property.Name]=ConvertTo-N1TransportValue $property.Value
        }
    }
    return $out
}
function ConvertTo-N1Json($Value) {
    ConvertTo-Json -InputObject (ConvertTo-N1TransportValue $Value) -Depth 20 -Compress
}
