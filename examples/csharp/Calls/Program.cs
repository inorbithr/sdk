// One program, one example per name: `dotnet run -- unary-get`. Each file beside this one
// is a single example and says what it shows in its header comment. The names are the
// call types listed in examples/README.md.
using System;
using System.Threading.Tasks;

var name = args.Length > 0 ? args[0] : "";
Func<Task>? example = name switch
{
    "unary-get" => UnaryGet.RunAsync,
    "list-pagination" => ListPagination.RunAsync,
    "stream-sse" => StreamSse.RunAsync,
    "stream-socket" => StreamSocket.RunAsync,
    "typed-error" => TypedError.RunAsync,
    "auth-token" => AuthToken.RunAsync,
    "auth-custom-token" => AuthCustomToken.RunAsync,
    "retries" => Retries.RunAsync,
    "concurrent-calls" => ConcurrentCalls.RunAsync,
    "config-load" => ConfigLoad.RunAsync,
    _ => null,
};
if (example is null)
{
    Console.Error.WriteLine("usage: dotnet run -- <unary-get|list-pagination|stream-sse|stream-socket|typed-error|auth-token|auth-custom-token|retries|concurrent-calls|config-load>");
    return 2;
}

await example();
return 0;
