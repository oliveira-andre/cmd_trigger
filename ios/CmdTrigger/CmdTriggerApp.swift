import SwiftUI

@main
struct CmdTriggerApp: App {
    @State private var model = AppModel.shared
    @Environment(\.scenePhase) private var phase

    var body: some Scene {
        WindowGroup {
            RootView()
                .environment(model)
                .preferredColorScheme(.dark)
                .tint(Theme.primary)
                .onOpenURL { model.open($0) }
                .onChange(of: phase) { _, now in
                    if now == .active { Task { await model.refresh() } }
                }
        }
    }
}

struct RootView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var model = model
        TabView {
            HomeView()
                .tabItem { Label("Home", systemImage: "square.grid.2x2") }
            ActivityView()
                .tabItem { Label("Activity", systemImage: "clock.arrow.circlepath") }
            SettingsView()
                .tabItem { Label("Settings", systemImage: "gearshape") }
        }
        .overlay(alignment: .top) {
            if let toast = model.toast {
                ToastView(toast: toast)
                    .padding(.top, 6)
                    .transition(.move(edge: .top).combined(with: .opacity))
                    .onTapGesture { model.toast = nil }
                    .task(id: toast.id) {
                        try? await Task.sleep(for: .seconds(toast.ok ? 2.5 : 5))
                        if model.toast?.id == toast.id { model.toast = nil }
                    }
            }
        }
        .animation(.spring(duration: 0.3), value: model.toast)
        .sheet(item: $model.shownRun) { RunDetail(run: $0) }
        .sheet(item: $model.pairRequest) { PairView(preset: $0) }
        .task { await model.refresh() }
    }
}
