param([long]$WindowId,[string]$Action='Buttons',[string]$Name,[string]$Value,[int]$Index=0,[int]$Width=0,[int]$Height=0)
$ErrorActionPreference='Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public class BiliProbe {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left,Top,Right,Bottom; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h,out Rect r);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h,int command);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h,IntPtr after,int x,int y,int width,int height,uint flags);
    [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h,uint message,UIntPtr w,IntPtr l);
}
'@
$handle=[IntPtr]$WindowId
[BiliProbe]::ShowWindow($handle,4) | Out-Null
$root=[System.Windows.Automation.AutomationElement]::FromHandle($handle)
switch($Action){
    'Buttons' {
        $root.FindAll([System.Windows.Automation.TreeScope]::Descendants,[System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ControlTypeProperty,[System.Windows.Automation.ControlType]::Button)) | ForEach-Object {$_.Current.Name}
    }
    'Invoke' {
        $elements=$root.FindAll([System.Windows.Automation.TreeScope]::Descendants,[System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::NameProperty,$Name))
        for($attempt=0;$attempt -lt 30 -and $elements.Count -le $Index;$attempt++){Start-Sleep -Milliseconds 100;$elements=$root.FindAll([System.Windows.Automation.TreeScope]::Descendants,[System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::NameProperty,$Name))}
        if($elements.Count -le $Index){throw "Button missing: $Name [$Index]"}
        $target=$elements[$Index]
        $pat=$null
        if ($target.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$pat)) {
            $pat.Invoke()
        } elseif ($target.TryGetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern, [ref]$pat)) {
            $pat.Toggle()
        } elseif ($target.TryGetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern, [ref]$pat)) {
            $pat.Select()
        } else {
            throw "Target $Name does not support Invoke, Toggle, or Select pattern"
        }
        Write-Output "Invoked $Name [$Index]"
    }
    'Input' {
        $elements=$root.FindAll([System.Windows.Automation.TreeScope]::Descendants,[System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::NameProperty,$Name))
        if($elements.Count -le $Index){throw "Input missing: $Name [$Index]"}
        ($elements[$Index].GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern)).SetValue($Value)
        Write-Output "Updated $Name [$Index]"
    }
    'Resize' { [BiliProbe]::SetWindowPos($handle,[IntPtr]::Zero,0,0,$Width,$Height,0x0016) | Out-Null; Write-Output "Resized $Width x $Height" }
    'Wheel' {
        $rect=[BiliProbe+Rect]::new();[BiliProbe]::GetWindowRect($handle,[ref]$rect) | Out-Null
        $x=$rect.Left+[int](($rect.Right-$rect.Left)*0.8);$y=$rect.Top+[int](($rect.Bottom-$rect.Top)*0.65)
        $wheel=[UIntPtr]([uint64](([int64](-120*[int]$Value) -shl 16) -band [long]4294967295))
        $point=[IntPtr]::new(($y -shl 16) -bor ($x -band 0xffff))
        [BiliProbe]::SendMessage($handle,0x020A,$wheel,$point) | Out-Null
        Write-Output 'Scrolled application content'
    }
    'Close' {($root.GetCurrentPattern([System.Windows.Automation.WindowPattern]::Pattern)).Close();Write-Output 'Closed test window'}
    default {throw 'Unknown UI probe action'}
}
