// `dotnet run --project src/App -- "your task"`: runs the loop with the scripted model and prints
// the outcome, the stop reason and what it used.
using App;

var task = args.Length == 0 ? "add 2 and 3" : string.Join(' ', args);
var run = new Agent(ScriptedModel.Demo(), Tools.All, Tools.Allow, new Budget(), log: Console.Error.WriteLine).Run(task);
Console.WriteLine($"[{ScriptedModel.Stub}]");
Console.WriteLine(run.Text ?? $"(stopped: {run.Stop.Name()})");
Console.WriteLine($"stop={run.Stop.Name()} steps={run.Steps} tool_calls={run.ToolCalls} tokens={run.Tokens}");
return run.Stop == Stop.Done ? 0 : 1;
