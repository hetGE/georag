// SPA Router — toggles page sections without full reload

(function () {
    const routes = {
        '/': { page: 'chat', title: 'geoRAG - Chat' },
        '/documents': { page: 'documents', title: 'geoRAG - Documents' },
    };

    function navigate(path) {
        const route = routes[path] || routes['/'];

        // Toggle page sections
        document.querySelectorAll('.spa-page').forEach(el => el.style.display = 'none');
        const target = document.getElementById('page-' + route.page);
        if (target) target.style.display = '';

        // Update body attribute
        document.body.dataset.activePage = route.page;

        // Update document title
        document.title = route.title;

        // Update nav active class
        document.querySelectorAll('nav ul:last-child a').forEach(a => {
            const href = a.getAttribute('href');
            a.classList.toggle('active', href === path || (href === '/' && path === '/'));
        });

        // Fire custom event for lazy-init (e.g. documents)
        document.dispatchEvent(new CustomEvent('spa:pageshow', { detail: { page: route.page } }));
    }

    // Intercept nav link clicks
    document.addEventListener('click', (e) => {
        const link = e.target.closest('nav a[href]');
        if (!link) return;
        const href = link.getAttribute('href');
        if (!routes[href]) return;

        e.preventDefault();
        if (location.pathname !== href) {
            history.pushState(null, '', href);
        }
        navigate(href);
    });

    // Browser back/forward
    window.addEventListener('popstate', () => {
        navigate(location.pathname);
    });

    // Initial route on page load
    navigate(location.pathname);
})();
