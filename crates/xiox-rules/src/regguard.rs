//! Chiavi e valori di registro che una regola esterna non può mai cancellare.
//!
//! Un solo controllo per tutte le fonti: la conversione di Winapp2, la
//! raccolta (`collect`, che copre anche CleanerML) e la pulizia, che lo rifà
//! prima di cancellare.

use crate::model::RegRoot;

/// `Ok(())` se la regola può cancellare la chiave (`value: None`) o il valore
/// `value` della chiave `path`; altrimenti `Err(motivo in italiano)`.
///
/// `path` è relativo alla radice (`Software\…`, senza `HKCU\`/`HKLM\`). Le due
/// radici seguono le stesse regole; `Wow6432Node` (vista a 32 bit, anche sotto
/// `Software\Classes`) si toglie prima del confronto. I nomi si confrontano
/// senza maiuscole e alla lettera: nel registro `*` è un nome come un altro
/// (`Software\Classes\*` è la chiave di tutti i file, non un jolly), e solo
/// `\` separa le chiavi.
///
/// Regole, nell'ordine:
/// - solo sotto `Software` (mai `System`, `SAM`, `SECURITY`, `Control Panel`…),
///   mai `..`, `.` o un nome di valore vuoto;
/// - le chiavi critiche (`INSIDE`: avvio automatico, Winlogon, criteri,
///   Defender, certificati, gestori dei file eseguibili…) non si toccano né
///   intere, né nelle sottochiavi, né nei valori; nemmeno un loro antenato si
///   cancella intero;
/// - le chiavi di sistema prese per intero (`WHOLE`, `…\CurrentVersion` e le
///   sue figlie dirette, `Software\Classes\*`, `CLSID`…) e i loro antenati non
///   si cancellano interi; le loro sottochiavi più profonde e i valori sì
///   (salvo i valori delle due `CurrentVersion`);
/// - profondità minima di una chiave intera: `Software\X`, e
///   `Software\Microsoft\X` / `Software\Classes\X`.
pub fn reg_allowed(root: RegRoot, path: &str, value: Option<&str>) -> Result<(), &'static str> {
    // Le due radici hanno le stesse regole (`Software` è la stessa struttura).
    match root {
        RegRoot::Hkcu | RegRoot::Hklm => {}
    }
    let comps = normalize(path)?;
    if comps.first().map(String::as_str) != Some("software") {
        return Err("chiave di registro fuori da Software");
    }
    if value.is_some_and(|v| v.is_empty()) {
        return Err("nome del valore di registro vuoto");
    }
    for (critical, reason) in INSIDE {
        let crit = split(critical);
        // La chiave critica stessa o una sua sottochiave (chiave o valore).
        if comps.starts_with(&crit) {
            return Err(reason);
        }
        // Un suo antenato cancellato intero la porterebbe via.
        if value.is_none() && crit.starts_with(&comps) {
            return Err(reason);
        }
    }
    if value.is_some() {
        if comps.len() < 2 {
            return Err(TOO_HIGH);
        }
        if NO_VALUES.iter().any(|k| comps == split(k)) {
            return Err("valore di una chiave di sistema di Windows");
        }
        return Ok(());
    }
    let min = match comps.get(1).map(String::as_str) {
        Some("microsoft" | "classes") => 3,
        _ => 2,
    };
    if comps.len() < min {
        return Err(TOO_HIGH);
    }
    if WHOLE.iter().any(|w| split(w).starts_with(&comps)) || is_current_version_child(&comps) {
        return Err("chiave di sistema presa per intero (o un suo antenato)");
    }
    Ok(())
}

const TOO_HIGH: &str = "chiave di registro troppo in alto per cancellarla intera";

/// Chiavi critiche con tutto ciò che contengono: né la chiave, né le
/// sottochiavi, né i valori, né un antenato intero.
const INSIDE: &[(&str, &str)] = &[
    // Avvio automatico.
    (r"software\microsoft\windows\currentversion\run", AUTORUN),
    (r"software\microsoft\windows\currentversion\runonce", AUTORUN),
    (r"software\microsoft\windows\currentversion\runonceex", AUTORUN),
    (r"software\microsoft\windows\currentversion\runservices", AUTORUN),
    (r"software\microsoft\windows\currentversion\runservicesonce", AUTORUN),
    (r"software\microsoft\windows\currentversion\shellserviceobjectdelayload", AUTORUN),
    (r"software\microsoft\windows\currentversion\explorer\shellexecutehooks", AUTORUN),
    (r"software\microsoft\windows\currentversion\explorer\browser helper objects", AUTORUN),
    (r"software\microsoft\windows\currentversion\shell extensions\approved", AUTORUN),
    (r"software\microsoft\active setup\installed components", AUTORUN),
    (r"software\microsoft\command processor", AUTORUN),
    // Criteri (anche `Policies\Explorer\Run`).
    (r"software\microsoft\windows\currentversion\policies", POLICIES),
    (r"software\policies", POLICIES),
    // Accesso e cuore di Windows NT.
    (r"software\microsoft\windows nt\currentversion\winlogon", LOGON),
    (r"software\microsoft\windows nt\currentversion\windows", LOGON),
    (r"software\microsoft\windows nt\currentversion\image file execution options", LOGON),
    (r"software\microsoft\windows nt\currentversion\silentprocessexit", LOGON),
    (r"software\microsoft\windows nt\currentversion\profilelist", LOGON),
    (r"software\microsoft\windows nt\currentversion\svchost", LOGON),
    (r"software\microsoft\windows nt\currentversion\drivers32", LOGON),
    (r"software\microsoft\windows nt\currentversion\fonts", LOGON),
    (r"software\microsoft\windows nt\currentversion\fontsubstitutes", LOGON),
    (r"software\microsoft\windows\currentversion\authentication", LOGON),
    (r"software\microsoft\windows\currentversion\explorer\shell folders", LOGON),
    (r"software\microsoft\windows\currentversion\explorer\user shell folders", LOGON),
    // Sicurezza.
    (r"software\microsoft\windows defender", SECURITY),
    (r"software\microsoft\cryptography", SECURITY),
    (r"software\microsoft\systemcertificates", SECURITY),
    (r"software\microsoft\enterprisecertificates", SECURITY),
    // Come si aprono i programmi: un gestore cancellato impedisce di avviarli.
    (r"software\classes\exefile", EXEC),
    (r"software\classes\.exe", EXEC),
    (r"software\classes\batfile", EXEC),
    (r"software\classes\.bat", EXEC),
    (r"software\classes\cmdfile", EXEC),
    (r"software\classes\.cmd", EXEC),
    (r"software\classes\comfile", EXEC),
    (r"software\classes\.com", EXEC),
    (r"software\classes\regfile", EXEC),
    (r"software\classes\.reg", EXEC),
    (r"software\classes\lnkfile", EXEC),
    (r"software\classes\.lnk", EXEC),
    (r"software\classes\msi.package", EXEC),
    (r"software\classes\.msi", EXEC),
    (r"software\classes\dllfile", EXEC),
    (r"software\classes\.dll", EXEC),
    (r"software\classes\piffile", EXEC),
    (r"software\classes\.pif", EXEC),
    (r"software\classes\scrfile", EXEC),
    (r"software\classes\.scr", EXEC),
    (r"software\classes\htafile", EXEC),
    (r"software\classes\.hta", EXEC),
    (r"software\classes\vbsfile", EXEC),
    (r"software\classes\.vbs", EXEC),
    (r"software\classes\jsfile", EXEC),
    (r"software\classes\.js", EXEC),
    (r"software\classes\wsffile", EXEC),
    (r"software\classes\.wsf", EXEC),
    (r"software\classes\.ps1", EXEC),
    (r"software\classes\microsoft.powershellscript.1", EXEC),
    // `*` alla lettera: i gestori di tutti i file (menu, anteprime, proprietà).
    (r"software\classes\*", EXEC),
    // Programmi predefiniti e installazioni di Windows Installer.
    (r"software\clients", DEFAULTS),
    (r"software\registeredapplications", DEFAULTS),
    (r"software\classes\installer", INSTALLER),
    (r"software\microsoft\windows\currentversion\installer", INSTALLER),
    (r"software\microsoft\windows\currentversion\explorer\startupapproved", AUTORUN),
];

const DEFAULTS: &str = "programmi predefiniti di Windows";
const INSTALLER: &str = "programmi installati (Windows Installer)";

const AUTORUN: &str = "avvio automatico dei programmi";
const POLICIES: &str = "criteri di sistema";
const LOGON: &str = "chiave di sistema di Windows (accesso, servizi, cartelle note)";
const SECURITY: &str = "sicurezza di Windows (Defender, certificati, crittografia)";
const EXEC: &str = "apertura dei file eseguibili";

/// Chiavi che non si cancellano intere (né i loro antenati); sottochiavi più
/// profonde e valori sì.
const WHOLE: &[&str] = &[
    r"software\microsoft\windows\currentversion",
    r"software\microsoft\windows nt\currentversion",
    r"software\microsoft\windows\shell",
    r"software\microsoft\internet explorer",
    r"software\classes\clsid",
    r"software\classes\interface",
    r"software\classes\typelib",
    r"software\classes\appid",
    r"software\classes\directory",
    r"software\classes\folder",
    r"software\classes\drive",
    r"software\classes\allfilesystemobjects",
    r"software\classes\systemfileassociations",
    r"software\classes\protocols",
    r"software\classes\mime",
    r"software\classes\local settings",
    r"software\classes\applications",
    r"software\classes\http",
    r"software\classes\https",
];

/// Chiavi i cui valori non si cancellano (cartelle di sistema, versione e
/// attivazione di Windows).
const NO_VALUES: &[&str] = &[
    r"software\microsoft\windows\currentversion",
    r"software\microsoft\windows nt\currentversion",
];

/// Figlia diretta di `Software\Microsoft\Windows[ NT]\CurrentVersion`
/// (`Run`, `Uninstall`, `App Paths`, `Explorer`, `Winlogon`…).
fn is_current_version_child(comps: &[String]) -> bool {
    comps.len() == 5
        && comps[1] == "microsoft"
        && matches!(comps[2].as_str(), "windows" | "windows nt")
        && comps[3] == "currentversion"
}

fn split(key: &str) -> Vec<String> {
    key.split('\\').map(str::to_string).collect()
}

/// Componenti in minuscolo, senza vuoti né `Wow6432Node` (subito sotto
/// `Software` o sotto `Software\Classes`).
fn normalize(path: &str) -> Result<Vec<String>, &'static str> {
    let mut comps: Vec<String> = Vec::new();
    for c in path.split('\\').filter(|c| !c.is_empty()) {
        if c == "." || c == ".." {
            return Err("il percorso di registro contiene «.» o «..»");
        }
        comps.push(c.to_lowercase());
    }
    loop {
        let at = match comps.get(1).map(String::as_str) {
            Some("wow6432node") => 1,
            Some("classes") if comps.get(2).map(String::as_str) == Some("wow6432node") => 2,
            _ => break,
        };
        comps.remove(at);
    }
    Ok(comps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use RegRoot::{Hkcu, Hklm};

    fn key(root: RegRoot, path: &str) -> Result<(), &'static str> {
        reg_allowed(root, path, None)
    }

    #[test]
    fn system_keys_probed_by_the_review_are_refused() {
        for (root, path) in [
            (Hklm, r"Software\Microsoft"),
            (Hkcu, r"Software\Microsoft"),
            (Hklm, r"Software\Microsoft\Windows"),
            (Hklm, r"Software\Microsoft\Windows NT"),
            (Hklm, r"Software\Microsoft\Windows NT\CurrentVersion"),
            (Hklm, r"Software\Microsoft\Windows NT\CurrentVersion\Winlogon"),
            (Hklm, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon"),
            (Hklm, r"Software\Microsoft\Windows NT\CurrentVersion\Windows"),
            (Hklm, r"Software\Policies"),
            (Hklm, r"Software\Policies\Google\Chrome"),
            (Hklm, r"Software\Classes"),
            (Hklm, r"Software\Classes\exefile"),
            (Hklm, r"Software\Classes\exefile\shell\open\command"),
            (Hkcu, r"Software\Classes\.exe"),
            (Hklm, r"Software\Classes\*"),
            (Hklm, r"Software\Classes\CLSID"),
            (Hklm, r"Software\Classes\Directory"),
            (Hklm, r"Software\Classes\Folder"),
            (Hklm, r"Software\Classes\AllFilesystemObjects"),
            (Hklm, r"Software\Classes\Drive"),
            (Hklm, r"Software\Classes\Interface"),
            (Hklm, r"Software\Classes\TypeLib"),
            (Hklm, r"Software\Classes\.dll"),
            (Hklm, r"Software\Classes\.lnk"),
            (Hklm, r"Software\Classes\.bat"),
            (Hklm, r"Software\Classes\.cmd"),
            (Hklm, r"Software\Classes\.com"),
            (Hklm, r"Software\Classes\.reg"),
            (Hklm, r"Software\Classes\.msi"),
            (Hkcu, r"Software\Microsoft\Windows\CurrentVersion"),
            (Hkcu, r"Software\Microsoft\Windows\CurrentVersion\Run"),
            (Hklm, r"Software\Microsoft\Windows\CurrentVersion\RunOnce"),
            (Hklm, r"Software\Microsoft\Windows\CurrentVersion\RunServices"),
            (Hklm, r"Software\Microsoft\Windows\CurrentVersion\Run\APR_DEACTIVATED"),
            (Hkcu, r"Software\Microsoft\Windows\CurrentVersion\Policies"),
            (Hkcu, r"Software\Microsoft\Windows\CurrentVersion\Policies\Explorer\Run"),
            (Hklm, r"Software\Microsoft\Windows\CurrentVersion\Uninstall"),
            (Hklm, r"Software\Microsoft\Windows\CurrentVersion\App Paths"),
            (Hkcu, r"Software\Microsoft\Windows\CurrentVersion\Explorer"),
            (Hkcu, r"Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders"),
            (Hklm, r"Software\Microsoft\Windows Defender"),
            (Hklm, r"Software\Microsoft\Windows Defender\Exclusions\Paths"),
            (Hklm, r"Software\Microsoft\Cryptography"),
            (Hklm, r"Software\Microsoft\SystemCertificates\Root"),
            (Hklm, r"System\CurrentControlSet\Services"),
            (Hklm, "System"),
            (Hklm, "SAM"),
            (Hklm, r"SECURITY\Policy"),
            (Hkcu, r"Control Panel\Desktop"),
            (Hklm, "Software"),
            (Hklm, r"Software\Wow6432Node"),
            (Hklm, r"Software\Wow6432Node\Microsoft"),
            (Hklm, r"Software\Wow6432Node\Classes"),
            (Hklm, r"Software\Wow6432Node\Microsoft\Windows\CurrentVersion\Run"),
            (Hkcu, r"Software\Classes\Wow6432Node\CLSID"),
            (Hkcu, r"Software\Foo\..\Microsoft"),
            (Hkcu, r"Software\\Microsoft\\"),
        ] {
            assert!(key(root, path).is_err(), "{root:?}\\{path} accettata");
        }
    }

    #[test]
    fn critical_values_are_refused() {
        for (root, path, value) in [
            (Hklm, r"Software\Microsoft\Windows NT\CurrentVersion\Winlogon", "Userinit"),
            (Hkcu, r"Software\Microsoft\Windows NT\CurrentVersion\Winlogon", "Shell"),
            (Hkcu, r"Software\Microsoft\Windows\CurrentVersion\Run", "OneDrive"),
            (Hklm, r"Software\Wow6432Node\Microsoft\Windows\CurrentVersion\Run", "x"),
            (Hkcu, r"Software\Microsoft\Windows\CurrentVersion\Policies\Explorer\Run", "x"),
            (Hklm, r"Software\Microsoft\Windows\CurrentVersion", "ProgramFilesDir"),
            (Hklm, r"Software\Microsoft\Windows NT\CurrentVersion", "DigitalProductId"),
            (Hklm, r"Software\Policies\Microsoft\Windows Defender", "DisableAntiSpyware"),
            (Hkcu, r"Software\Classes\exefile\shell\open\command", "x"),
            (Hklm, r"Software\Microsoft\Command Processor", "AutoRun"),
            (Hklm, r"System\CurrentControlSet\Control", "x"),
            (Hkcu, "Software", "Foo"),
            (Hkcu, r"Software\Foo", ""),
        ] {
            assert!(
                reg_allowed(root, path, Some(value)).is_err(),
                "{root:?}\\{path}|{value} accettato"
            );
        }
    }

    #[test]
    fn legit_winapp2_keys_and_values_stay_allowed() {
        for (root, path) in [
            (Hkcu, r"Software\Microsoft\Windows\CurrentVersion\Explorer\RecentDocs"),
            (Hkcu, r"Software\Microsoft\Windows\CurrentVersion\Explorer\RunMRU"),
            (Hkcu, r"Software\Microsoft\Windows\CurrentVersion\Explorer\ComDlg32\OpenSavePidlMRU"),
            (Hklm, r"Software\Classes\.mpf"),
            (Hklm, r"Software\Classes\MPR.DocHostUIHandler"),
            (Hklm, r"Software\Classes\CLSID\{3F2BBC05-40DF-11D2-9455-00104BC936FF}"),
            (Hkcu, r"Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\BagMRU"),
            (Hkcu, r"Software\Classes\Wow6432Node\Local Settings\Software\Microsoft\Windows\Shell\Bags"),
            (Hklm, r"Software\Microsoft\Tracing"),
            (Hkcu, r"Software\Microsoft\Internet Explorer\TypedURLs"),
            (Hkcu, r"Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Compatibility Assistant\Store"),
            (Hklm, r"Software\Microsoft\Windows\CurrentVersion\Uninstall\OldTool"),
            (Hkcu, r"Software\VOS"),
            (Hklm, r"Software\Wow6432Node\VOS"),
            (Hkcu, r"Software\Vendor\App\Recent"),
        ] {
            assert_eq!(key(root, path), Ok(()), "{root:?}\\{path} rifiutata");
        }
        for (root, path, value) in [
            (Hkcu, r"Software\Microsoft\Windows\CurrentVersion\Explorer", "ScreenshotIndex"),
            (Hklm, r"Software\Microsoft\Windows\CurrentVersion\Setup", "Installation Sources"),
            (Hkcu, r"Software\7-Zip\FM", "FolderHistory"),
            (Hkcu, r"Software\Microsoft", "Foo"),
        ] {
            assert_eq!(reg_allowed(root, path, Some(value)), Ok(()), "{path}|{value} rifiutato");
        }
    }

    /// M3: altre chiavi critiche, intere, con le sottochiavi e i valori.
    #[test]
    fn more_critical_keys_are_refused() {
        for (root, path, value) in [
            (Hklm, r"Software\Clients", None),
            (Hklm, r"Software\Clients\StartMenuInternet\Firefox", None),
            (Hkcu, r"Software\RegisteredApplications", Some("Firefox")),
            (Hklm, r"Software\Classes\Installer\Products\ABC", None),
            (Hklm, r"Software\Microsoft\Windows\CurrentVersion\Installer\UserData", None),
            (Hklm, r"Software\Microsoft\Windows\CurrentVersion\Installer\UserData\S-1-5-18", None),
            (Hkcu, r"Software\Classes\htafile\shell\open\command", None),
            (Hkcu, r"Software\Classes\VBSFile", None),
            (Hkcu, r"Software\Classes\.ps1", None),
            (Hkcu, r"Software\Classes\*\shellex\ContextMenuHandlers\OldTool", None),
            (Hkcu, r"Software\Classes\*\shell\x", Some("y")),
            (Hkcu, r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run", Some("OneDrive")),
            (Hkcu, r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run32", Some("x")),
        ] {
            assert!(reg_allowed(root, path, value).is_err(), "{root:?}\\{path}|{value:?} accettato");
        }
        // Le chiavi vicine restano.
        assert_eq!(key(Hkcu, r"Software\Classes\.mpf"), Ok(()));
        assert_eq!(key(Hkcu, r"Software\Microsoft\Windows\CurrentVersion\Explorer\RecentDocs"), Ok(()));
    }

    #[test]
    fn reasons_are_italian_and_specific() {
        assert_eq!(
            reg_allowed(Hklm, r"Software\Microsoft\Windows NT\CurrentVersion\Winlogon", Some("Userinit")),
            Err(LOGON)
        );
        assert_eq!(key(Hkcu, r"Software\Microsoft\Windows\CurrentVersion\Run"), Err(AUTORUN));
        assert_eq!(key(Hklm, r"System\Setup"), Err("chiave di registro fuori da Software"));
        assert_eq!(key(Hklm, r"Software\Microsoft"), Err(AUTORUN)); // antenato di Run
        assert_eq!(key(Hklm, r"Software\Classes"), Err(EXEC));
        assert_eq!(key(Hklm, r"Software\Classes\*"), Err(EXEC));
    }
}
