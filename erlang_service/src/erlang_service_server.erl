%%==============================================================================
%% The server responsible for co-ordinating work
%%==============================================================================
-module(erlang_service_server).

%%==============================================================================
%% Behaviours
%%==============================================================================
-behaviour(gen_server).

%%==============================================================================
%% Exports
%%==============================================================================

-export([start_link/0]).

%% gen_server callbacks
-export([
    init/1,
    handle_call/3,
    handle_cast/2,
    handle_info/2
]).

%% API
-export([ get_docs/2, ct_info/1, elp_lint/3 ]).

%%==============================================================================
%% Includes
%%==============================================================================

%%==============================================================================
%% Macros
%%==============================================================================
-define(SERVER, ?MODULE).

%%==============================================================================
%% Type Definitions
%%==============================================================================
-type state() :: #{io := erlang:group_leader(), requests := [request()]}.
-type request() :: {pid(), pos_integer(), reference() | infinity}.

%%==============================================================================
%% API
%%==============================================================================
-spec start_link() -> {ok, pid()}.
start_link() ->
    process_flag(trap_exit, true),
    gen_server:start_link({local, ?SERVER}, ?MODULE, noargs, []).

get_docs(Data, DocOrigin) ->
    gen_server:cast(?SERVER, {request, get_docs, Data, [DocOrigin]}).

ct_info(Data) ->
    gen_server:cast(?SERVER, {request, ct_info, Data, []}).

elp_lint(Data, PostProcess, Deterministic) ->
    gen_server:cast(?SERVER, {request, elp_lint, Data, [PostProcess, Deterministic]}).

%%==============================================================================
%% gen_server callbacks
%%==============================================================================
-spec init(noargs) -> {ok, state()}.
init(noargs) ->
    State = #{io => erlang:group_leader(), requests => []},
    {ok, State}.

-spec handle_call(any(), any(), state()) -> {reply, any(), state()}.
handle_call(_Request, _From, State) ->
    {reply, ok, State}.

-spec handle_cast(any(), state()) -> {noreply, state()}.
handle_cast({request, Request, Data, AdditionalParams}, #{requests := Requests} = State) ->
    {Pid, Id} = process_request_async(callback_module(Request), Data, AdditionalParams),
    Timer = case timeout(Request) of
                infinity ->
                    infinity;
                Timeout ->
                    erlang:send_after(Timeout, ?SERVER, {timeout, Pid})
            end,
    {noreply, State#{requests => [{Pid, Id, Timer}|Requests]}};
handle_cast({result, Id, Result}, #{io := IO, requests := Requests} = State) ->
    case lists:keytake(Id, 2, Requests) of
        {value, {Pid, Id, infinity}, NewRequests} ->
            reply(Id, Result, IO),
            {noreply, State#{requests => NewRequests}};
        {value, {Pid, Id, Timer}, NewRequests} ->
            erlang:cancel_timer(Timer),
            reply(Id, Result, IO),
            {noreply, State#{requests => NewRequests}};
        _ ->
            {noreply, State}
    end;
handle_cast({exception, Id, ExceptionData}, #{io := IO, requests := Requests} = State) ->
    case lists:keytake(Id, 2, Requests) of
        {value, {Pid, Id, infinity}, NewRequests} ->
            reply_exception(Id, ExceptionData, IO),
            {noreply, State#{requests => NewRequests}};
        {value, {Pid, Id, Timer}, NewRequests} ->
            erlang:cancel_timer(Timer),
            reply_exception(Id, ExceptionData, IO),
            {noreply, State#{requests => NewRequests}};
        _ ->
            {noreply, State}
    end.

-spec handle_info(any(), state()) -> {noreply, state()}.
handle_info({timeout, Pid}, #{io := IO, requests := Requests} = State) ->
    case lists:keytake(Pid, 1, Requests) of
        {value, {Pid, Id, Timer}, NewRequests} ->
            exit(Pid, normal),
            reply_exception(Id, <<"Timeout">>, IO),
            {noreply, State#{requests => NewRequests}};
        _ ->
            {noreply, State}
    end.

%%==============================================================================
%% Internal Functions
%%==============================================================================
reply(Id, Segments, Device) ->
    %% Use file:write/2 since it writes bytes
    BinId = integer_to_binary(Id),
    Size = integer_to_binary(length(Segments)),
    Data = [encode_segment(Segment) || Segment <- Segments],
    file:write(Device, [<<"REPLY ">>, BinId, $\s, Size, $\n | Data]),
    ok.

reply_exception(Id, Data, Device) ->
    %% Use file:write/2 since it writes bytes
    Size = integer_to_binary(byte_size(Data)),
    BinId = integer_to_binary(Id),
    file:write(Device, [<<"EXCEPTION ">>, BinId, $\s, Size, $\n | Data]),
    ok.

encode_segment({Tag, Data}) ->
    Size = integer_to_binary(byte_size(Data)),
    [Tag, $\s, Size, $\n | Data].

process_request_async(Module, Data, AdditionalParams) ->
    [Id|Params] = binary_to_term(Data),
    Pid = spawn_link(
        fun() ->
            try
                case Module:run(Params ++ AdditionalParams) of
                    {ok, Result} ->
                        gen_server:cast(?SERVER, {result, Id, Result});
                    {error, Error} ->
                        gen_server:cast(?SERVER, {exception, Id, Error})
                end
            catch
                Class:Reason:StackTrace ->
                    Formatted = erl_error:format_exception(Class, Reason, StackTrace),
                    ExceptionData = unicode:characters_to_binary(Formatted),
                    gen_server:cast(?SERVER, {exception, Id, ExceptionData})
            end
        end),
    {Pid, Id}.

callback_module(get_docs) -> erlang_service_edoc;
callback_module(ct_info) -> erlang_service_ct;
callback_module(elp_lint) -> erlang_service_lint.

timeout(ct_info) -> 2500;
timeout(_) -> infinity.