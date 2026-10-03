import * as vscode from "vscode";
import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
    TransportKind,
} from "vscode-languageclient/node";

let client: LanguageClient | undefined;

export function activate(context: vscode.ExtensionContext): void {
    const config = vscode.workspace.getConfiguration("rasmalai");
    const serverPath = config.get<string>("serverPath", "rnx");
    const serverOptions: ServerOptions = {
        command: serverPath,
        args: ["lsp"],
        transport: TransportKind.stdio,
    };
    const clientOptions: LanguageClientOptions = {
        documentSelector: [{ scheme: "file", language: "rasmalai" }],
        synchronize: {
            fileEvents: vscode.workspace.createFileSystemWatcher("**/*.rnx"),
        },
    };
    client = new LanguageClient("rasmalai", "Rasmalai Language Server", serverOptions, clientOptions);
    client.start();
    context.subscriptions.push({ dispose: () => client?.stop() });
}

export function deactivate(): Thenable<void> | undefined {
    if (client === undefined) {
        return undefined;
    }
    return client.stop();
}
