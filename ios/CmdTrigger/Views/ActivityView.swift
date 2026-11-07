import SwiftUI

/// Recent runs on your computers, from this phone and from everywhere else.
struct ActivityView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        NavigationStack {
            ScrollView {
                LazyVStack(spacing: 8) {
                    Text("Activity")
                        .font(.system(size: 24, weight: .bold))
                        .tracking(-0.6)
                        .foregroundStyle(Theme.strong)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.top, 8)
                        .padding(.bottom, 6)
                    if model.runs.isEmpty {
                        Text("Nothing has run yet. Runs from the app, Siri, schedules, webhooks and everything else show here.")
                            .font(.system(size: 14))
                            .foregroundStyle(Theme.muted)
                            .card()
                    }
                    ForEach(model.runs) { run in
                        Button { model.shownRun = run } label: { row(run) }
                            .buttonStyle(PressStyle())
                    }
                }
                .padding(.horizontal, 16)
                .padding(.bottom, 24)
            }
            .background(Theme.canvas)
            .refreshable { await model.refreshRuns() }
            .toolbar(.hidden, for: .navigationBar)
            .task { await model.refreshRuns() }
        }
    }

    private func row(_ run: RunItem) -> some View {
        HStack(alignment: .top, spacing: 12) {
            Circle().fill(run.ok ? Theme.up : run.status == .running || run.status == .queued ? Theme.pending : Theme.down)
                .frame(width: 8, height: 8)
                .padding(.top, 6)
            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline) {
                    Text(run.name).font(.system(size: 15, weight: .semibold)).foregroundStyle(Theme.strong).lineLimit(1)
                    if let params = run.params {
                        Text("“\(params)”").font(.system(size: 13)).foregroundStyle(Theme.mutedStrong).lineLimit(1)
                    }
                    Spacer()
                    Text(Format.ago(run.at)).font(.system(size: 12)).foregroundStyle(Theme.muted)
                }
                Text("\(run.computer) · \(run.source)\(run.exitCode.map { $0 == 0 ? "" : " · exit \($0)" } ?? "")")
                    .font(.system(size: 12))
                    .foregroundStyle(Theme.muted)
                if !run.output.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    Text(run.output.trimmingCharacters(in: .whitespacesAndNewlines))
                        .font(.system(size: 12, design: .monospaced))
                        .foregroundStyle(Theme.mutedStrong)
                        .lineLimit(2)
                        .multilineTextAlignment(.leading)
                }
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .hairline()
    }
}
