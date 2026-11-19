// The loop. One model, a set of tools, and three limits you can read in one place:
//
// - a budget: steps, tool calls and tokens; a run that would pass one stops and says which;
// - an allow-list: a tool that is not on it is refused, and its code never runs;
// - approval: a tool marked NeedsApproval runs only when someone answers yes.
//
// Every stop has a name (Run.Stop), so a caller never has to guess why a run ended. The log
// holds what happened (tool names, outcomes, counts), never a tool's arguments or results:
// those can be customer data.
namespace App;

/// <summary>Why a run stopped.</summary>
public enum Stop
{
    Done,
    BudgetSteps,
    BudgetToolCalls,
    BudgetTokens,
    AwaitingApproval,
}

public static class StopNames
{
    public static string Name(this Stop stop) => stop switch
    {
        Stop.Done => "done",
        Stop.BudgetSteps => "budget_steps",
        Stop.BudgetToolCalls => "budget_tool_calls",
        Stop.BudgetTokens => "budget_tokens",
        _ => "awaiting_approval",
    };
}

public sealed record Budget(int MaxSteps = 8, int MaxToolCalls = 4, int MaxTokens = 4000);

public sealed record ToolCall(string Name, IReadOnlyDictionary<string, object> Args);

/// <summary>What the model answered: text (it is done) or a tool call, and the tokens it used.</summary>
public sealed record Turn(string? Text = null, ToolCall? Call = null, int Tokens = 0);

/// <summary>
/// Anything with Next(history). Wrap your provider here, signed in the way your provider's
/// subscription allows; this template never holds a key.
/// </summary>
public interface IModel
{
    Turn Next(IReadOnlyList<Message> history);
}

public sealed record Tool(string Name, Func<IReadOnlyDictionary<string, object>, object?> Fn, bool NeedsApproval = false);

/// <summary>One entry of the history: a user task, an assistant answer or a tool outcome.</summary>
public sealed record Message(string Role, string? Name = null, string? Content = null, object? Result = null, string? Error = null);

public sealed record Run(Stop Stop, string? Text, int Steps, int ToolCalls, int Tokens, IReadOnlyList<Message> History);

public sealed class Agent(
    IModel model,
    IEnumerable<Tool> tools,
    ISet<string> allow,
    Budget budget,
    Func<ToolCall, bool?>? approve = null,
    Action<string>? log = null)
{
    private readonly Dictionary<string, Tool> _tools = tools.ToDictionary(t => t.Name);

    private void Log(string line) => log?.Invoke(line);

    public Run Run(string task)
    {
        var history = new List<Message> { new("user", Content: task) };
        int steps = 0, toolCalls = 0, tokens = 0;

        App.Run Stopped(Stop reason, string? text = null)
        {
            Log($"{{\"event\":\"stop\",\"reason\":\"{reason.Name()}\",\"steps\":{steps},\"tool_calls\":{toolCalls},\"tokens\":{tokens}}}");
            return new App.Run(reason, text, steps, toolCalls, tokens, history);
        }

        while (true)
        {
            if (steps >= budget.MaxSteps) return Stopped(Stop.BudgetSteps);
            var turn = model.Next(history);
            steps++;
            if (tokens + turn.Tokens > budget.MaxTokens) return Stopped(Stop.BudgetTokens); // this turn is dropped
            tokens += turn.Tokens;
            if (turn.Call is null)
            {
                history.Add(new("assistant", Content: turn.Text ?? ""));
                return Stopped(Stop.Done, turn.Text ?? "");
            }
            var call = turn.Call;
            if (toolCalls >= budget.MaxToolCalls) return Stopped(Stop.BudgetToolCalls);
            var shown = call.Name.Length > 64 ? call.Name[..64] : call.Name;
            if (!_tools.TryGetValue(call.Name, out var tool) || !allow.Contains(call.Name))
            {
                Log($"{{\"event\":\"refused\",\"tool\":\"{shown}\",\"why\":\"not_allowed\"}}");
                history.Add(new("tool", Name: call.Name, Error: "not_allowed"));
                continue;
            }
            if (tool.NeedsApproval)
            {
                var answer = approve?.Invoke(call); // null (no answer) stops the run
                if (answer is null)
                {
                    Log($"{{\"event\":\"approval\",\"tool\":\"{shown}\",\"answer\":\"none\"}}");
                    return Stopped(Stop.AwaitingApproval);
                }
                if (answer == false)
                {
                    Log($"{{\"event\":\"approval\",\"tool\":\"{shown}\",\"answer\":\"no\"}}");
                    history.Add(new("tool", Name: call.Name, Error: "denied"));
                    continue;
                }
                Log($"{{\"event\":\"approval\",\"tool\":\"{shown}\",\"answer\":\"yes\"}}");
            }
            var result = tool.Fn(call.Args);
            toolCalls++;
            Log($"{{\"event\":\"tool\",\"tool\":\"{shown}\",\"ok\":true}}");
            history.Add(new("tool", Name: call.Name, Result: result));
        }
    }
}
