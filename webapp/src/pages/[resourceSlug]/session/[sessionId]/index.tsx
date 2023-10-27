import React, { useState, useEffect, useRef } from 'react';
// import { useParams } from 'next/navigation';
import Head from 'next/head';
import * as API from '../../../../api';
import { useAccountContext } from '../../../../context/account';
import { useSocketContext } from '../../../../context/socket';
import { useChatContext } from '../../../../context/chat';
import { useRouter } from 'next/router';
import { Message } from '../../../../components/chat/message';
import SessionChatbox from '../../../../components/SessionChatbox';
import classNames from '../../../../components/ClassNames';
// import { toast } from 'react-toastify';

export default function Session(props) {

	const [accountContext]: any = useAccountContext();
	const { account, csrf } = accountContext as any;

	const router = useRouter();	
	const [state, dispatch] = useState(props);
	const [error, setError] = useState();
	const { sessionId } = props.query || router.query;
	const { session } = state;

	const [_chatContext, setChatContext]: any = useChatContext();
	useEffect(() => {
		setChatContext(session ? {
			prompt: session.prompt,
			status: session.status,
		} : null);
	}, [session]);

	const [isAtBottom, setIsAtBottom] = useState(true);
	const scrollContainerRef = useRef(null);
	useEffect(() => {
		if (!scrollContainerRef || !scrollContainerRef.current) { return; }
		const handleScroll = () => {
			const { scrollTop, scrollHeight, clientHeight } = scrollContainerRef.current;
			// Check if scrolled to the bottom
			const isCurrentlyAtBottom = scrollTop + clientHeight >= scrollHeight;
			if (isCurrentlyAtBottom !== isAtBottom) {
				setIsAtBottom(isCurrentlyAtBottom);
			}
		};
		const container = scrollContainerRef.current;
		container.addEventListener('scroll', handleScroll);
		// Cleanup
		return () => {
			container.removeEventListener('scroll', handleScroll);
		};
	}, [isAtBottom]);

	const socketContext = useSocketContext();
	const [messages, setMessages] = useState(null);
	const [terminated, setTerminated] = useState(null);
	const sentLastMessage = !messages || (messages.length > 0 && messages[messages.length-1].incoming);
	const lastMessageFeedback = !messages || (messages.length > 0 && messages[messages.length-1].isFeedback);
	const chatBusyState = sentLastMessage || !lastMessageFeedback;
	async function joinSessionRoom() {
		socketContext.emit('join_room', sessionId);
	}
	function handleTerminateMessage(message) {
		console.log('Received terminate message', message);
		setTerminated(true);
		//TODO: set terminates state
	}
	function handleSocketMessage(message) {
		console.log(message);
		if (!message) {return;}
		const newMessage = typeof message === 'string'
			? { type: null, text: message }
			: message;
		setMessages(oldMessages => {
			return oldMessages
				.concat([newMessage])
				.sort((ma, mb) => ma.ts - mb.ts);
		});
	}
	function handleSocketStatus(status) {
		console.log('chat status', status);
		if (!status) {return;}
		setChatContext({
			prompt: session.prompt,
			status,
		});
	}
	function scrollToBottom() {
		//scroll to bottom when messages added (if currently at bottom)
		if (scrollContainerRef && scrollContainerRef.current && isAtBottom) {
			setTimeout(() => {
				scrollContainerRef.current.scrollTo({
					left: 0,
					top: scrollContainerRef.current.scrollHeight,
					behavior: 'smooth',
				});
			}, 250);
		}
	}
	useEffect(() => {
		scrollToBottom();
	}, [messages]);
	function handleJoinedRoom() {
		if (messages.length === 0) {
			//|| !messages.find(m => m.incoming === false)) {
			//if no messagesfound, session is new so submit the messages and one to task queue
			socketContext.emit('message', {
				room: sessionId,
				authorName: account.name,
				incoming: true,
				message: {
					type: 'text',
					text: session.prompt,
				}
			});
			socketContext.emit('message', {
				room: 'task_queue',
				event: session.type,
				message: {
					task: session.prompt,
					sessionId,
				},
			});
		}
		scrollToBottom();
	}
	function handleSocketStart() {
		socketContext.on('connect', joinSessionRoom);
		socketContext.on('terminate', handleTerminateMessage);
		// socketContext.on('reconnect', joinSessionRoom);
		socketContext.on('message', handleSocketMessage);
		socketContext.on('status', handleSocketStatus);
		socketContext.on('joined', handleJoinedRoom);
		socketContext.connected ? joinSessionRoom() : socketContext.connect();
	}
	function handleSocketStop() {
		socketContext.off('connect', joinSessionRoom);
		// socketContext.off('reconnect', joinSessionRoom);
		socketContext.off('terminate', handleTerminateMessage);
		socketContext.off('joined', handleJoinedRoom);
		socketContext.off('message', handleSocketMessage);
		socketContext.off('status', handleSocketStatus);
		// socketContext.connected && socketContext.disconnect();
	}
	useEffect(() => {
		if (session) {
			setTerminated(session.status === 'terminated');
			//todo: move enums out of db file (to exclude backend mongo import stuff), then use in frontend)
		}
	}, [session]);
	useEffect(() => {
		API.getSession({
			resourceSlug: account.currentTeam,
			sessionId,
		}, dispatch, setError, router);
		API.getMessages({
			resourceSlug: account.currentTeam,
			sessionId,
		}, (_messages) => {
			setMessages(_messages.map(m => m.message));
		}, setError, router);
	}, []);
	useEffect(() => {
		//once we have the session and messages (or empty message array is fine), start
		if (session && messages != null) {
			handleSocketStart();
			return () => {
				//stop/disconnect on unmount
				handleSocketStop();
			};
		}
	}, [session, messages]);

	function sendMessage(e) {
		e.preventDefault();
		const message: string = e.target.prompt ? e.target.prompt.value : e.target.value;
		if (!message || message.trim().length === 0) { return; }
		socketContext.emit('message', {
			room: sessionId,
			authorName: account.name,
			incoming: true,
			message: {
				type: 'text',
				text: message,
			}
		});
		e.target.reset ? e.target.reset() : e.target.form.reset();
	}

	if (!session || messages == null) {
		return 'Loading...'; //TODO: loader
	}

	return (
		<>

			<Head>
				<title>Session - {sessionId}</title>
			</Head>

			{/*<div className='border-b pb-2 my-2 mb-6'>
				<h3 className='pl-2 font-semibold text-gray-900'>Session {sessionId}</h3>
			</div>*/}

			<div className='flex flex-col -m-7 -my-10 flex flex-col flex-1'>

				<div className='overflow-y-auto' ref={scrollContainerRef}>
					{messages && messages.map((m, mi, marr) => {
						return <Message
							key={`message_${mi}`}
							prevMessage={mi > 0 ? marr[mi-1] : null}
							message={m.message.text}
							messageType={m.message?.type}
							messageLanguage={m.message?.language}
							authorName={m.authorName}
							incoming={m.incoming}
							ts={m.ts}
							isFeedback={m.isFeedback}
						/>;
					})}
					{chatBusyState && !terminated && <div className='text-center border-t pb-6 pt-8 mt-4'>
						<span className='inline-block animate-bounce ad-100 h-4 w-2 mx-1 rounded-full bg-indigo-600 opacity-75'></span>
						<span className='inline-block animate-bounce ad-300 h-4 w-2 mx-1 rounded-full bg-indigo-600 opacity-75'></span>
						<span className='inline-block animate-bounce ad-500 h-4 w-2 mx-1 rounded-full bg-indigo-600 opacity-75'></span>
					</div>}
				</div>

				<div className='flex flex-row justify-center border-t p-4 mt-auto'>
					<div className='flex items-start space-x-4 basis-1/2'>
						<div className='flex-shrink-0  ring-1 rounded-full ring-gray-300'>
							<span
								className='inline-block h-10 w-10 text-center pt-2 font-bold'
							>
								{account.name.charAt(0).toUpperCase()}
							</span>
						</div>
						<div className='min-w-0 flex-1 h-full'>
							{terminated 
								? <p className='text-center h-full me-14 pt-3'>This session was terminated.</p>
								: <SessionChatbox chatBusyState={chatBusyState} onSubmit={sendMessage} />}
						</div>
					</div>
				</div>

			</div>

		</>
	);

};

export async function getServerSideProps({ req, res, query, resolvedUrl, locale, locales, defaultLocale }) {
	return JSON.parse(JSON.stringify({ props: res?.locals?.data }));
};
