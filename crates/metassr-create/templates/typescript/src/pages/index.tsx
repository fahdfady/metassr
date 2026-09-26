import React, { useState } from 'react';
import metacallLogo from "../../static/assets/metacall-logo.png"

import { Footer } from '../components/footer';
import Clock from '../components/clock';

export default function Index() {
	const [count, setCount] = useState(0)

	return (
		<main className="page">
			<img className="logo" src={metacallLogo} alt="MetaCall" />

			<h1 className="heroTitle">%NAME%</h1>
			<p className="heroLead">%DESC%</p>

			<section className="benchmarks" aria-labelledby="demo-title">
				<h2 id="demo-title" className="sectionTitle">Live demo</h2>
				<p className="sectionLead"><Clock /></p>
				<p className="sectionLead">
					<button className="button" onClick={() => setCount((count) => count + 1)}>
						Increase ({count})
					</button>
				</p>
			</section>

			<section className="benchmarks" aria-labelledby="run-title">
				<h2 id="run-title" className="sectionTitle">Build &amp; run</h2>
				<p className="sectionLead">
					<code>$ metassr dev</code> ·{' '}
					<code>$ metassr build -t ssr</code> ·{' '}
					<code>$ metassr start</code>
				</p>
			</section>

			<Footer />
		</main>
	)
}