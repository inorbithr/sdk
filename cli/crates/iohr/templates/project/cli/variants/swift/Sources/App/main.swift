import Cli
import Foundation

// Passes the real streams; the exit code is the result of run().
exit(
    run(
        Array(CommandLine.arguments.dropFirst()),
        out: { FileHandle.standardOutput.write(Data($0.utf8)) },
        err: { FileHandle.standardError.write(Data($0.utf8)) }))
