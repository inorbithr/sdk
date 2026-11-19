import @@MODULE@@
import Foundation

// Exit 0 when the bar is met, 1 when missed, 2 when the dataset or the bar is wrong.
var output = StandardStream.out
var errors = StandardStream.err
exit(runSuite(arguments: Array(CommandLine.arguments.dropFirst()), out: &output, err: &errors))
