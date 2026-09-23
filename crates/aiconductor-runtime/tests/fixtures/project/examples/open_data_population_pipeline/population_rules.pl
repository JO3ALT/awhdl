% Deterministic population-trend classification used by the end-to-end test.

trend_class(Slope, strong_decline) :-
    Slope =< -300000, !.
trend_class(Slope, decline) :-
    Slope < 0, !.
trend_class(Slope, growth) :-
    Slope > 0, !.
trend_class(_, stable).

attention_rule(ChangePercent, CagrPercent, RSquared, attention_required) :-
    ChangePercent =< -5.0,
    CagrPercent < 0.0,
    RSquared >= 0.70, !.
attention_rule(_, _, _, monitor_only).

population_assessment(Slope, ChangePercent, CagrPercent, RSquared, Trend, Attention) :-
    trend_class(Slope, Trend),
    attention_rule(ChangePercent, CagrPercent, RSquared, Attention).
