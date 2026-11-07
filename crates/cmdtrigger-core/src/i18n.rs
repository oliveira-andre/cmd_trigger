//! Tray and dialog strings in the original's seven languages. The windows carry their own
//! table in the frontend; this one covers what Rust draws (the tray menu).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    Es,
    Pt,
    De,
    Fr,
    Jp,
    It,
}

impl Lang {
    pub const ALL: [Lang; 7] = [Lang::En, Lang::Es, Lang::Pt, Lang::De, Lang::Fr, Lang::Jp, Lang::It];

    pub fn from_code(code: &str) -> Self {
        match code.trim().to_lowercase().as_str() {
            "es" => Lang::Es,
            "pt" => Lang::Pt,
            "de" => Lang::De,
            "fr" => Lang::Fr,
            "jp" | "ja" => Lang::Jp,
            "it" => Lang::It,
            _ => Lang::En,
        }
    }

    /// The code written to `language.cfg`.
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Es => "es",
            Lang::Pt => "pt",
            Lang::De => "de",
            Lang::Fr => "fr",
            Lang::Jp => "jp",
            Lang::It => "it",
        }
    }

    /// The language's own name, as listed in the Language menu.
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Es => "Español",
            Lang::Pt => "Português",
            Lang::De => "Deutsch",
            Lang::Fr => "Français",
            Lang::Jp => "日本語",
            Lang::It => "Italiano",
        }
    }
}

/// Look up a tray string. Unknown keys come back as the key itself.
pub fn t(lang: Lang, key: &'static str) -> &'static str {
    let row: Option<[&'static str; 7]> = match key {
        "Version" => Some(["Version", "Versión", "Versão", "Version", "Version", "バージョン", "Versione"]),
        "Open CmdTrigger" => Some([
            "Open CmdTrigger…",
            "Abrir CmdTrigger…",
            "Abrir o CmdTrigger…",
            "CmdTrigger öffnen…",
            "Ouvrir CmdTrigger…",
            "CmdTrigger を開く…",
            "Apri CmdTrigger…",
        ]),
        "Run" => Some(["Run", "Ejecutar", "Executar", "Ausführen", "Exécuter", "実行", "Esegui"]),
        "Commands" => Some(["Commands", "Comandos", "Comandos", "Befehle", "Commandes", "コマンド", "Comandi"]),
        "Automations" => Some([
            "Automations",
            "Automatizaciones",
            "Automações",
            "Automationen",
            "Automatisations",
            "オートメーション",
            "Automazioni",
        ]),
        "Nothing yet" => Some([
            "Nothing yet",
            "Nada todavía",
            "Nada ainda",
            "Noch nichts",
            "Rien pour l'instant",
            "まだありません",
            "Ancora niente",
        ]),
        "Server: connected" => Some([
            "Server: connected",
            "Servidor: conectado",
            "Servidor: conectado",
            "Server: verbunden",
            "Serveur : connecté",
            "サーバー: 接続済み",
            "Server: connesso",
        ]),
        "Server: connecting…" => Some([
            "Server: connecting…",
            "Servidor: conectando…",
            "Servidor: conectando…",
            "Server: verbinde…",
            "Serveur : connexion…",
            "サーバー: 接続中…",
            "Server: connessione…",
        ]),
        "Server: offline" => Some([
            "Server: offline",
            "Servidor: sin conexión",
            "Servidor: offline",
            "Server: offline",
            "Serveur : hors ligne",
            "サーバー: オフライン",
            "Server: offline",
        ]),
        "Server: sign in again" => Some([
            "Server: sign in again",
            "Servidor: vuelve a iniciar sesión",
            "Servidor: entre novamente",
            "Server: neu anmelden",
            "Serveur : reconnectez-vous",
            "サーバー: 再サインイン",
            "Server: accedi di nuovo",
        ]),
        "Server: not set up" => Some([
            "Server: not set up",
            "Servidor: sin configurar",
            "Servidor: não configurado",
            "Server: nicht eingerichtet",
            "Serveur : non configuré",
            "サーバー: 未設定",
            "Server: non configurato",
        ]),
        "Background Service" => Some([
            "Background Service",
            "Servicio en segundo plano",
            "Serviço em segundo plano",
            "Hintergrunddienst",
            "Service en arrière-plan",
            "バックグラウンドサービス",
            "Servizio in background",
        ]),
        "Install Background Service" => Some([
            "Install Background Service",
            "Instalar el servicio",
            "Instalar o serviço",
            "Dienst installieren",
            "Installer le service",
            "サービスをインストール",
            "Installa il servizio",
        ]),
        "Remove Background Service" => Some([
            "Remove Background Service",
            "Quitar el servicio",
            "Remover o serviço",
            "Dienst entfernen",
            "Supprimer le service",
            "サービスを削除",
            "Rimuovi il servizio",
        ]),
        "Language" => Some(["Language", "Idioma", "Idioma", "Sprache", "Langue", "言語", "Lingua"]),
        "Quit" => Some([
            "Quit CmdTrigger",
            "Salir de CmdTrigger",
            "Sair do CmdTrigger",
            "CmdTrigger beenden",
            "Quitter CmdTrigger",
            "CmdTrigger を終了",
            "Esci da CmdTrigger",
        ]),
        _ => None,
    };
    let index = Lang::ALL.iter().position(|l| *l == lang).unwrap_or(0);
    row.map(|r| r[index]).unwrap_or(key)
}
