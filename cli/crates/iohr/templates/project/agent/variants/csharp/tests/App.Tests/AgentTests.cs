// The passes-on-init test: a scripted run within budget; a run that would pass a budget stops
// with a named reason; a tool outside the allow-list is refused and never runs; a tool that
// needs approval does not run without an answer.
using App;
using Xunit;

namespace App.Tests;

public class AgentTests
{
    private static Run Ask(IEnumerable<Turn> turns, IEnumerable<Tool>? tools = null, ISet<string>? allow = null,
        Budget? budget = null, Func<ToolCall, bool?>? approve = null, Action<string>? log = null) =>
        new Agent(new ScriptedModel(turns), tools ?? Tools.All, allow ?? Tools.Allow, budget ?? new Budget(), approve, log)
            .Run("task");

    private static Turn Add(int tokens) =>
        new(Call: new ToolCall("add", new Dictionary<string, object> { ["a"] = 1, ["b"] = 1 }), Tokens: tokens);

    private static Turn CallOf(string name, string key, object value) =>
        new(Call: new ToolCall(name, new Dictionary<string, object> { [key] = value }));

    [Fact]
    public void AScriptedRunFinishesWithinTheBudget()
    {
        var run = new Agent(ScriptedModel.Demo(), Tools.All, Tools.Allow, new Budget()).Run("add 2 and 3");
        Assert.Equal(Stop.Done, run.Stop);
        Assert.Equal("2 + 3 = 5", run.Text);
        Assert.Equal(1, run.ToolCalls);
        Assert.True(run.Steps <= new Budget().MaxSteps);
        Assert.Equal(5, run.History[1].Result);
    }

    [Fact]
    public void ARunThatWouldPassABudgetStopsAndNamesIt()
    {
        var forever = Enumerable.Repeat(Add(10), 50).ToList();
        Assert.Equal(Stop.BudgetToolCalls, Ask(forever, budget: new Budget(8, 3, 4000)).Stop);
        Assert.Equal(Stop.BudgetSteps, Ask(forever, budget: new Budget(2, 9, 4000)).Stop);
        var heavy = Ask(Enumerable.Repeat(Add(3000), 5), budget: new Budget(8, 9, 4000));
        Assert.Equal(Stop.BudgetTokens, heavy.Stop);
        Assert.True(heavy.Tokens <= 4000);
    }

    [Fact]
    public void AToolOutsideTheAllowListIsRefusedAndNeverRuns()
    {
        var ran = 0;
        var tools = Tools.All.Append(new Tool("shell", _ => { ran++; return null; })).ToList();
        var run = Ask([CallOf("shell", "cmd", "rm -rf /"), new Turn(Text: "ok")], tools);
        Assert.Equal(0, ran);
        Assert.Equal("not_allowed", run.History[1].Error);
        Assert.Equal(Stop.Done, run.Stop);
        Assert.Equal(0, run.ToolCalls);
    }

    [Fact]
    public void AToolThatNeedsApprovalDoesNotRunWithoutAnAnswer()
    {
        var ran = 0;
        var tools = new[] { new Tool("delete_note", _ => { ran++; return null; }, NeedsApproval: true) };
        var allow = new HashSet<string> { "delete_note" };
        Turn[] script = [CallOf("delete_note", "id", "n1"), new Turn(Text: "ok")];
        Assert.Equal(Stop.AwaitingApproval, Ask(script, tools, allow).Stop);
        Assert.Equal(Stop.AwaitingApproval, Ask(script, tools, allow, approve: _ => null).Stop);
        var denied = Ask(script, tools, allow, approve: _ => false);
        Assert.Equal(Stop.Done, denied.Stop);
        Assert.Equal("denied", denied.History[1].Error);
        Assert.Equal(0, ran);
        var yes = Ask(script, tools, allow, approve: _ => true);
        Assert.Equal(Stop.Done, yes.Stop);
        Assert.Equal(1, ran);
    }

    [Fact]
    public void TheLogHoldsWhatHappenedAndNeverAToolArgument()
    {
        var lines = new List<string>();
        var args = new Dictionary<string, object> { ["a"] = 424242, ["b"] = 1 };
        Ask([new Turn(Call: new ToolCall("add", args)), new Turn(Text: "secret-answer")], log: lines.Add);
        var text = string.Join('\n', lines);
        Assert.Contains("\"event\":\"tool\"", text);
        Assert.DoesNotContain("424242", text);
        Assert.DoesNotContain("secret-answer", text);
    }
}
